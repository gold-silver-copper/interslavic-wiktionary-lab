#!/usr/bin/env python3
"""Validate issue-89 cross-layer linguistic identity/confidence invariants."""

import csv
import json
import sys
from collections import defaultdict
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else "site")
official_path = Path(sys.argv[2] if len(sys.argv) > 2 else "data/official-isv.csv")
entries = json.loads((root / "entries.json").read_text())
by_title = defaultdict(list)
for entry in entries:
    by_title[entry["title"].strip().lower()].append(entry)
    historical = {"cu", "orv"}.intersection(entry.get("langs_list", []))
    assert not historical, ("historical hint published as modern evidence", entry["id"], historical)
    if not entry["official"]:
        assert entry["prob"] is None, (
            "uncalibrated corpus score published as probability", entry["id"], entry["prob"]
        )


def aspect(pos_raw: str):
    if "ipf./pf." in pos_raw:
        return "ipf/pf"
    if "ipf." in pos_raw:
        return "ipf"
    if "pf." in pos_raw:
        return "pf"
    return None


expected_aspects = defaultdict(set)
official_spellings = set()
with official_path.open(newline="") as handle:
    for row in csv.DictReader(handle):
        title = row["isv"].strip()
        if not title or "#" in title:
            continue
        key = title.lower()
        official_spellings.add(key)
        value = aspect(row["partOfSpeech"])
        if value:
            expected_aspects[key].add(value)

missing = sorted(
    title
    for title in official_spellings
    if not any(entry["official"] for entry in by_title.get(title, []))
)
assert not missing, ("exact official spellings missing from export", missing[:20], len(missing))

for title, values in expected_aspects.items():
    pages = [entry for entry in by_title[title] if entry["official"]]
    assert pages, ("aspect lemma missing exact page", title)
    expected = "ipf/pf" if "ipf/pf" in values or len(values) > 1 else next(iter(values))
    assert any(page["aspect"] == expected for page in pages), (
        "exact-spelling aspect mismatch", title, expected, [(p["id"], p["aspect"]) for p in pages]
    )

by_id = {entry["id"]: entry for entry in entries}
lemmas = json.loads((root / "api/lemmas.json").read_text())["lemmas"]
for row in lemmas:
    entry = by_id.get(row[4])
    if row[2] == "generated" and entry is not None and not entry["official"]:
        assert row[3] is None, (
            "uncalibrated corpus lemma API probability", row[:6]
        )

proposal_lines = (root / "novel-words.tsv").read_text().splitlines()
proposal_header = "form\tpos\tprobability\tbucket\tancestor\tn_langs\tn_branches\tgloss"
assert proposal_lines == [proposal_header], (
    "uncalibrated or malformed novel-word proposal artifact", proposal_lines[:2]
)

print(
    f"linguistic logic valid: {len(official_spellings)} exact official spellings, "
    f"{len(expected_aspects)} aspect spellings, no historical confidence leaks, "
    "no cross-domain probabilities/proposals"
)
