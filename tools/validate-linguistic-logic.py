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
        # V11 item 5 (issue #90): the corpus-coverage calibrator is committed,
        # so generated entries carry its probability — which must be a proper
        # open-interval value (None only when the calibrator file was absent).
        assert entry["prob"] is None or (
            isinstance(entry["prob"], float) and 0.0 < entry["prob"] < 1.0
        ), ("corpus probability out of range", entry["id"], entry["prob"])


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


def citation_forms(raw: str):
    forms = []
    start = 0
    depth = 0
    parts = []
    for i, ch in enumerate(raw):
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth = max(0, depth - 1)
        elif ch == "," and depth == 0:
            parts.append(raw[start:i])
            start = i + 1
    parts.append(raw[start:])
    for part in parts:
        part = part.strip()
        if not part or "#" in part or "!" in part:
            continue
        while True:
            match = re.search(r"\([^)]*\)", part)
            if not match:
                break
            part = (part[:match.start()] + part[match.end():]).strip()
        part = part.split(",", 1)[0].strip()
        if not part or any(marker in part for marker in ("#", "!", "*", "(", ")")):
            continue
        if part not in forms:
            forms.append(part)
    return forms


expected_senses = {}
expected_aspects = {}
official_spellings = set()
with official_path.open(newline="") as handle:
    for row in csv.DictReader(handle):
        byforms = [form.lower() for form in citation_forms(row["isv"])]
        if not byforms:
            continue
        official_spellings.update(byforms)
        gloss = row["en"].strip()
        sense_id = row["id"]
        pos = normalized_pos(row["partOfSpeech"])
        expected_senses[sense_id] = (set(byforms), gloss, pos)
        value = aspect(row["partOfSpeech"])
        if value:
            expected_aspects[sense_id] = (set(byforms), gloss, pos, value)

lemmas = json.loads((root / "api/lemmas.json").read_text())["lemmas"]
official_api_lemmas = {
    row[0].strip().lower()
    for row in lemmas
    if row[2] in {"official", "official-only"}
}
official_entry_titles = {
    entry["title"].strip().lower()
    for entry in entries
    if entry["official"]
}
represented_spellings = official_entry_titles | official_api_lemmas
missing = sorted(title for title in official_spellings if title not in represented_spellings)
assert not missing, ("official byform spellings missing from export/API", missing[:20], len(missing))

actual_senses = {}
actual_entries = {}
for entry in entries:
    if not entry["official"]:
        assert entry.get("official_id") is None, ("generated entry has official sense ID", entry["id"])
        continue
    sense_id = entry.get("official_id")
    assert sense_id, ("official entry lacks source sense ID", entry["id"])
    assert sense_id not in actual_senses, ("duplicate official source sense ID", sense_id)
    expected = expected_senses.get(sense_id)
    assert expected is not None, ("unexpected official source sense ID", sense_id)
    expected_byforms, expected_gloss, expected_pos = expected
    actual_title = entry["title"].strip().lower()
    actual = (entry["gloss"].strip(), entry["pos"])
    assert actual_title in expected_byforms, (
        "official source title is not a citation byform", sense_id, actual_title, sorted(expected_byforms)
    )
    assert actual == (expected_gloss, expected_pos), (
        "official source sense metadata differs", sense_id, (expected_gloss, expected_pos), actual
    )
    actual_senses[sense_id] = actual
    actual_entries[sense_id] = entry
assert set(actual_senses) == set(expected_senses), (
    "official source sense IDs differ", sorted(set(expected_senses) ^ set(actual_senses))[:20]
)

for sense_id, expected in expected_aspects.items():
    entry = actual_entries[sense_id]
    expected_byforms, expected_gloss, expected_pos, expected_aspect = expected
    actual_title = entry["title"].strip().lower()
    actual = (entry["gloss"].strip(), entry["pos"], entry["aspect"])
    assert actual_title in expected_byforms, (
        "official aspect title is not a citation byform", sense_id, actual_title, sorted(expected_byforms)
    )
    assert actual == (expected_gloss, expected_pos, expected_aspect), (
        "official aspect sense mismatch", sense_id, (expected_gloss, expected_pos, expected_aspect), actual
    )

by_id = {entry["id"]: entry for entry in entries}
assert len(by_id) == len(entries), "duplicate entries.json IDs"
for row in lemmas:
    entry_id = row[4]
    if entry_id != 0:
        assert entry_id in by_id, ("API lemma references missing entry", row[:6])
    entry = by_id.get(entry_id)
    if row[2] == "generated" and entry is not None and not entry["official"]:
        # V11 item 5 (issue #90): corpus reconstructions may now carry a
        # probability from the committed corpus-coverage calibrator. It must
        # be a proper open-interval probability — an exact 0.0/1.0 or an
        # out-of-range value means a broken calibrator, and None means the
        # calibrator file was absent at export time.
        assert row[3] is None or (isinstance(row[3], float) and 0.0 < row[3] < 1.0), (
            "corpus lemma probability out of range", row[:6]
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
proposal_header = ("form\tpos\tprobability\tbucket\tancestor\tn_langs\tn_branches\tgloss"
                   "\tclassification\tofficial")
assert proposal_lines and proposal_lines[0] == proposal_header, (
    "malformed novel-word proposal header", proposal_lines[:1]
)
# V11 item 5: with the corpus-coverage calibrator committed, proposals are
# live again. Each row must carry a calibrated probability in the review
# band or above, and the bucket must agree with the probability.
for line in proposal_lines[1:]:
    cols = line.split("\t")
    assert len(cols) == 10, ("malformed proposal row", line)
    prob = float(cols[2])
    assert 0.3 <= prob < 1.0, ("proposal probability out of band", line)
    assert cols[3] == ("predlog" if prob >= 0.6 else "pregled"), (
        "proposal bucket disagrees with probability", line
    )
    # V12 item 3 + issue #99 invariant: a proposal is never an official
    # byform spelling, and near-official rows must cite their lemma.
    assert cols[8] in ("novel", "near-official"), ("bad classification", line)
    assert (cols[8] == "near-official") == bool(cols[9]), (
        "classification/official-lemma mismatch", line
    )
    # (Exact-spelling level here; the exporter's own filter additionally
    # excludes FOLD-level duplicates before rows are ever written.)
    assert cols[0].lower() not in official_spellings, (
        "proposal duplicates an official byform (issue #99 invariant)", line
    )

print(
    f"linguistic logic valid: {len(expected_senses)} official senses across "
    f"{len(official_spellings)} spellings, {len(expected_aspects)} aspect senses, "
    "no historical confidence leaks or encoded reconstruction residue, "
    "no cross-domain probabilities/proposals"
)
