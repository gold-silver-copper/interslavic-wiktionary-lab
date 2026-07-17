#!/usr/bin/env python3
"""Validate cross-layer linguistic identity and model-domain invariants."""

import csv
import hashlib
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else "site")
official_path = Path(sys.argv[2] if len(sys.argv) > 2 else "data/official-isv.csv")
lemmas_path = Path("data/slavic-lemmas.cache.json")
artifact_path = Path("data/corpus-coverage-calibration.json")
artifact = json.loads(artifact_path.read_text())
assert artifact["schema_version"] == 1
assert artifact["score_domain"] == "corpus-coverage-score-v1"
assert artifact["score_model_version"] == "coverage-languages-branches-v1"
assert artifact["label_policy_version"] == "official-pos-semantic-proxy-v1"
assert artifact["split_policy"] == "fnv1a-id-mod-4-holdout-v1"
assert artifact["algorithm_version"] == "decile-pava-train-only-v1"
sha256 = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
assert artifact["provenance"]["lemmas_sha256"] == sha256(lemmas_path), "stale lemma digest"
assert artifact["provenance"]["official_sha256"] == sha256(official_path), "stale official digest"
deciles = artifact["deciles"]
assert len(deciles) == 10 and all(0 <= p <= 1 for p in deciles)
assert all(a <= b for a, b in zip(deciles, deciles[1:])), "non-monotone corpus deciles"
counts = artifact["counts"]
bins = artifact["reliability_bins"]
assert len(bins) == 10
assert sum(row["count"] for row in bins) == counts["holdout"]
assert sum(row["hits"] for row in bins) == counts["holdout_positive"]
for key, threshold in (("propose", .6), ("review", .3)):
    op = artifact[key]
    assert op["threshold"] == threshold
    assert 0 < op["selected"] <= counts["holdout"]
    assert 0 <= op["hits"] <= op["selected"]
    assert abs(op["precision"] - op["hits"] / op["selected"]) < 1e-12
    assert abs(op["coverage"] - op["hits"] / counts["holdout_positive"]) < 1e-12

# Refit from the committed audit inventory to catch tampered deciles/metrics and
# prove holdout labels never enter fitting.
with Path("target/eval/corpus-coverage-evaluation.tsv").open(newline="") as handle:
    inventory = list(csv.DictReader(handle, delimiter="\t"))
train = [r for r in inventory if r["split"] == "train"]
holdout = [r for r in inventory if r["split"] == "holdout"]
assert len(train) == counts["train"] and len(holdout) == counts["holdout"]
assert sum(int(r["label"]) for r in train) == counts["train_positive"]
assert sum(int(r["label"]) for r in holdout) == counts["holdout_positive"]
train_bins = [[0, 0] for _ in range(10)]
for row in train:
    bucket = min(int(float(row["raw_score"]) * 10), 9)
    train_bins[bucket][0] += 1
    train_bins[bucket][1] += int(row["label"])
pools = []
for i, (n, hits) in enumerate(train_bins):
    if not n: continue
    pools.append([i, i, n, hits / n])
    while len(pools) >= 2 and pools[-2][3] > pools[-1][3]:
        _, last, w2, m2 = pools.pop()
        first, _, w1, m1 = pools.pop()
        pools.append([first, last, w1 + w2, (w1*m1 + w2*m2)/(w1+w2)])
refit = [None] * 10
for first, last, _, mean in pools:
    for i in range(first, last + 1): refit[i] = mean
previous = next(v for v in refit if v is not None)
for i, value in enumerate(refit):
    if value is None: refit[i] = previous
    else: previous = value
assert all(abs(a-b) < 1e-12 for a, b in zip(refit, deciles)), "artifact deciles differ from train-only refit"
for row in inventory:
    expected = deciles[min(int(float(row["raw_score"]) * 10), 9)]
    assert abs(float(row["probability"]) - expected) < 1e-6, ("inventory probability mismatch", row["concept_id"])

def inventory_metrics(rows, calibrated):
    bs = [[0, 0.0, 0] for _ in range(10)]
    brier = 0.0
    for row in rows:
        p = float(row["probability"] if calibrated else row["raw_score"])
        y = int(row["label"])
        brier += (p-y)**2
        bucket = min(int(p*10), 9)
        bs[bucket][0] += 1; bs[bucket][1] += p; bs[bucket][2] += y
    ece = sum(n/len(rows) * abs(total/n - hits/n) for n,total,hits in bs if n)
    return ece, brier/len(rows), bs
raw_ece, raw_brier, _ = inventory_metrics(holdout, False)
cal_ece, cal_brier, cal_bins = inventory_metrics(holdout, True)
assert abs(raw_ece-artifact["raw_holdout"]["ece"]) < 1e-6
assert abs(raw_brier-artifact["raw_holdout"]["brier"]) < 1e-6
assert abs(cal_ece-artifact["calibrated_holdout"]["ece"]) < 1e-6
assert abs(cal_brier-artifact["calibrated_holdout"]["brier"]) < 1e-6
for expected, actual in zip(cal_bins, bins):
    assert expected[0] == actual["count"] and expected[2] == actual["hits"]
for key in ("propose", "review"):
    op = artifact[key]
    selected = [r for r in holdout if float(r["probability"]) >= op["threshold"]]
    assert len(selected) == op["selected"] and sum(int(r["label"]) for r in selected) == op["hits"]

entries = json.loads((root / "entries.json").read_text())
by_title = defaultdict(list)
url_escape = re.compile(r"%[0-9A-Fa-f]{2}")
for entry in entries:
    by_title[entry["title"].strip().lower()].append(entry)
    for field in ("title", "ancestor"):
        assert not url_escape.search(entry.get(field, "")), (
            "URL-escaped transport bytes leaked into linguistic text", entry["id"], field
        )
    historical = {"cu", "orv"}.intersection(entry.get("langs_list", []))
    assert not historical, ("historical hint published as modern evidence", entry["id"], historical)
    assert entry["prob"] is None, (
        "official-match proxy leaked into entry probability", entry["id"], entry["prob"]
    )


def aspect(pos_raw: str):
    if "ipf./pf." in pos_raw: return "ipf/pf"
    if "ipf." in pos_raw: return "ipf"
    if "pf." in pos_raw: return "pf"
    return None


def normalized_pos(pos_raw: str):
    if aspect(pos_raw): return "verb"
    value = pos_raw.strip().lower()
    prefixes = [
        (("adj",), "adj"), (("adv",), "adv"), (("num",), "num"),
        (("pron",), "pron"), (("prep", "postp"), "prep"), (("conj",), "conj"),
        (("intj",), "intj"), (("particle", "prtcl"), "particle"),
        (("prefix",), "prefix"), (("suffix",), "suffix"), (("phrase",), "phrase"),
    ]
    if value in {"proper noun", "proper_noun", "name"}: return "proper_noun"
    for starts, result in prefixes:
        if value.startswith(starts): return result
    if value.startswith(("m.", "f.", "n.", "m/")) or value in {"m", "f", "n"}: return "noun"
    if value.startswith("v.") or value.startswith("v ") or value == "v": return "verb"
    return "other"


expected_senses, expected_aspects, official_spellings, proposal_exclusion_senses = {}, {}, set(), set()
with official_path.open(newline="") as handle:
    for row in csv.DictReader(handle):
        title = row["isv"].strip()
        if not title or "#" in title: continue
        key, gloss, sense_id = title.lower(), row["en"].strip(), row["id"]
        official_spellings.add(key)
        pos = normalized_pos(row["partOfSpeech"])
        expected_senses[sense_id] = (key, gloss, pos)
        if " " not in title and "#" not in title:
            proposal_exclusion_senses.add(sense_id)
        value = aspect(row["partOfSpeech"])
        if value: expected_aspects[sense_id] = (key, gloss, pos, value)

missing = sorted(title for title in official_spellings if not any(e["official"] for e in by_title.get(title, [])))
assert not missing, ("exact official spellings missing from export", missing[:20], len(missing))
actual_senses, actual_entries = {}, {}
for entry in entries:
    if not entry["official"]:
        assert entry.get("official_id") is None
        continue
    sense_id = entry.get("official_id")
    assert sense_id and sense_id not in actual_senses
    actual_senses[sense_id] = (entry["title"].strip().lower(), entry["gloss"].strip(), entry["pos"])
    actual_entries[sense_id] = entry
assert actual_senses == expected_senses, ("official source senses differ", list((expected_senses.items() ^ actual_senses.items()))[:20])
for sense_id, expected in expected_aspects.items():
    entry = actual_entries[sense_id]
    assert (entry["title"].strip().lower(), entry["gloss"].strip(), entry["pos"], entry["aspect"]) == expected

by_id = {entry["id"]: entry for entry in entries}
assert len(by_id) == len(entries), "duplicate entries.json IDs"
lemmas = json.loads((root / "api/lemmas.json").read_text())["lemmas"]
for row in lemmas:
    entry = by_id.get(row[4])
    if row[4] != 0: assert entry is not None, ("API lemma references missing entry", row[:6])
    if row[2] in {"official", "official-only", "grammar"}:
        assert row[3] is None, ("official/grammar API probability must be null", row[:6])
    elif row[2] == "generated" and entry is not None and not entry["official"]:
        assert row[3] is None, ("official-match proxy leaked into corpus API probability", row[:6])
    elif row[2] == "generated":
        assert row[3] is not None and 0 <= row[3] <= .9, ("invalid derivative Wilson probability", row[:6])

# Folded official citation keys come from the exported router itself, avoiding a
# second hand-maintained orthography table in this validator.
official_fold_keys = set()
for shard_path in sorted((root / "api/forms").glob("*.json")):
    records = json.loads(shard_path.read_text()).get("records", {})
    for key, analyses in records.items():
        for row in analyses:
            if row[2] != 0: assert row[2] in by_id
            entry = by_id.get(row[2])
            if (row[5] == "lemma" and row[6] in {"official", "official-only"}
                    and entry and entry.get("official_id") in proposal_exclusion_senses):
                official_fold_keys.add(key)

historical_names = ("starocŕkovnoslovjansky", "starovȯstočnoslovjansky")
for proto_path in sorted((root / "proto").glob("*.html")):
    for section in proto_path.read_text().split("<section"):
        if not any(name in section for name in historical_names): continue
        marker = section.find("proto-historical-hints")
        assert marker >= 0
        assert not any(name in section[:marker] for name in historical_names)

proposal_header = "form\tpos\tcoverage_proxy\tbucket\tancestor\tn_langs\tn_branches\tgloss"
with (root / "novel-words.tsv").open(newline="") as handle:
    reader = csv.DictReader(handle, delimiter="\t")
    assert "\t".join(reader.fieldnames or []) == proposal_header
    proposals = list(reader)
assert proposals, "calibrated proposal worklist is empty"
for row in proposals:
    proxy = float(row["coverage_proxy"])
    assert proxy >= .3 and row["bucket"] == ("predlog" if proxy >= .6 else "pregled")

# The proposal form itself is a folded router key for its citation-form API row.
# Compare as a multiset after looking up generated entries and excluding every
# key occupied by an official citation form, exactly as export does.
generated_key_by_id = {}
for shard_path in sorted((root / "api/forms").glob("*.json")):
    records = json.loads(shard_path.read_text()).get("records", {})
    for key, analyses in records.items():
        for row in analyses:
            if row[5] == "lemma" and row[6] == "generated" and row[2] in by_id and not by_id[row[2]]["official"]:
                generated_key_by_id[row[2]] = key
expected = Counter()
for entry in entries:
    key = generated_key_by_id.get(entry["id"])
    raw = min(.99, max(.05, .12 + .55 * min(entry["langs"], 8) / 8 + .33 * min(entry["branches"], 3) / 3))
    proxy = deciles[min(int(raw * 10), 9)]
    if entry["official"] or proxy < .3 or not key or key in official_fold_keys: continue
    if " " in entry["title"] or len(entry["title"]) < 3: continue
    expected[(entry["title"], entry["pos"], f'{proxy:.3f}', "predlog" if proxy >= .6 else "pregled",
              entry["ancestor"], str(entry["langs"]), str(entry["branches"]), entry["gloss"].replace("\t", " ").replace("\n", " "))] += 1
actual = Counter((r["form"], r["pos"], r["coverage_proxy"], r["bucket"], r["ancestor"], r["n_langs"], r["n_branches"], r["gloss"]) for r in proposals)
assert actual == expected, ("proposal multiset differs from eligible generated entries", list((expected - actual).items())[:5], list((actual - expected).items())[:5])

print(f"linguistic logic valid: {len(expected_senses)} official senses, {len(proposals)} corpus proposals; calibration/input/API/proposal invariants hold")
