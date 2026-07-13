#!/usr/bin/env python3
"""Validate issue-89 cross-layer linguistic identity/confidence invariants."""

import csv
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else "site")
official_path = Path(sys.argv[2] if len(sys.argv) > 2 else "data/official-isv.csv")
entries = json.loads((root / "entries.json").read_text())
by_title = defaultdict(list)
url_escape = re.compile(r"%[0-9A-Fa-f]{2}")
for entry in entries:
    by_title[entry["title"].strip().lower()].append(entry)
    for field in ("title", "ancestor"):
        assert not url_escape.search(entry.get(field, "")), (
            "URL-escaped transport bytes leaked into linguistic text",
            entry["id"], field, entry.get(field)
        )
    historical = {"cu", "orv"}.intersection(entry.get("langs_list", []))
    assert not historical, ("historical hint published as modern evidence", entry["id"], historical)
    if not entry["official"]:
        assert entry["prob"] is None, (
            "uncalibrated corpus score published as probability", entry["id"], entry["prob"]
        )


def normalized_pos(pos_raw: str):
    if aspect(pos_raw):
        return "verb"
    value = pos_raw.strip().lower()
    prefixes = [
        (("adj",), "adj"), (("adv",), "adv"), (("num",), "num"),
        (("pron",), "pron"), (("prep", "postp"), "prep"),
        (("conj",), "conj"), (("intj",), "intj"),
        (("particle", "prtcl"), "particle"), (("prefix",), "prefix"),
        (("suffix",), "suffix"), (("phrase",), "phrase"),
    ]
    if value in {"proper noun", "proper_noun", "name"}:
        return "proper_noun"
    for starts, result in prefixes:
        if value.startswith(starts):
            return result
    if value.startswith(("m.", "f.", "n.", "m/")) or value in {"m", "f", "n"}:
        return "noun"
    if value.startswith("v.") or value.startswith("v ") or value == "v":
        return "verb"
    return "other"


def aspect(pos_raw: str):
    if "ipf./pf." in pos_raw:
        return "ipf/pf"
    if "ipf." in pos_raw:
        return "ipf"
    if "pf." in pos_raw:
        return "pf"
    return None


expected_senses = {}
expected_aspects = {}
official_spellings = set()
with official_path.open(newline="") as handle:
    for row in csv.DictReader(handle):
        title = row["isv"].strip()
        if not title or "#" in title:
            continue
        key = title.lower()
        official_spellings.add(key)
        gloss = row["en"].strip()
        sense_id = row["id"]
        pos = normalized_pos(row["partOfSpeech"])
        expected_senses[sense_id] = (key, gloss, pos)
        value = aspect(row["partOfSpeech"])
        if value:
            expected_aspects[sense_id] = (key, gloss, pos, value)

missing = sorted(
    title
    for title in official_spellings
    if not any(entry["official"] for entry in by_title.get(title, []))
)
assert not missing, ("exact official spellings missing from export", missing[:20], len(missing))

actual_senses = {}
actual_entries = {}
for entry in entries:
    if not entry["official"]:
        assert entry.get("official_id") is None, ("generated entry has official sense ID", entry["id"])
        continue
    sense_id = entry.get("official_id")
    assert sense_id, ("official entry lacks source sense ID", entry["id"])
    assert sense_id not in actual_senses, ("duplicate official source sense ID", sense_id)
    actual_senses[sense_id] = (
        entry["title"].strip().lower(), entry["gloss"].strip(), entry["pos"]
    )
    actual_entries[sense_id] = entry
assert actual_senses == expected_senses, (
    "official source senses differ", list((expected_senses.items() ^ actual_senses.items()))[:20]
)

for sense_id, expected in expected_aspects.items():
    entry = actual_entries[sense_id]
    actual = (entry["title"].strip().lower(), entry["gloss"].strip(), entry["pos"], entry["aspect"])
    assert actual == expected, ("official aspect sense mismatch", sense_id, expected, actual)

by_id = {entry["id"]: entry for entry in entries}
assert len(by_id) == len(entries), "duplicate entries.json IDs"
lemmas = json.loads((root / "api/lemmas.json").read_text())["lemmas"]
for row in lemmas:
    entry_id = row[4]
    if entry_id != 0:
        assert entry_id in by_id, ("API lemma references missing entry", row[:6])
    entry = by_id.get(entry_id)
    if row[2] == "generated" and entry is not None and not entry["official"]:
        assert row[3] is None, (
            "uncalibrated corpus lemma API probability", row[:6]
        )

for shard_path in sorted((root / "api/forms").glob("*.json")):
    records = json.loads(shard_path.read_text()).get("records", {})
    for analyses in records.values():
        for row in analyses:
            entry_id = row[2]
            if entry_id != 0:
                assert entry_id in by_id, (
                    "API form references missing entry", shard_path.name, row[:5]
                )

historical_names = ("starocŕkovnoslovjansky", "starovȯstočnoslovjansky")
for proto_path in sorted((root / "proto").glob("*.html")):
    for section in proto_path.read_text().split("<section"):
        if not any(name in section for name in historical_names):
            continue
        marker = section.find("proto-historical-hints")
        assert marker >= 0, ("historical proto descendant is unlabeled", proto_path.name)
        assert not any(name in section[:marker] for name in historical_names), (
            "historical proto descendant appears as modern branch evidence", proto_path.name
        )

proposal_lines = (root / "novel-words.tsv").read_text().splitlines()
proposal_header = "form\tpos\tprobability\tbucket\tancestor\tn_langs\tn_branches\tgloss"
assert proposal_lines == [proposal_header], (
    "uncalibrated or malformed novel-word proposal artifact", proposal_lines[:2]
)

print(
    f"linguistic logic valid: {len(expected_senses)} official senses across "
    f"{len(official_spellings)} spellings, {len(expected_aspects)} aspect senses, "
    "no historical confidence leaks or encoded reconstruction residue, "
    "no cross-domain probabilities/proposals"
)
