#!/usr/bin/env python3
"""Validate issue-75 production export invariants (CI guard)."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else "site")
entries = json.loads((root / "entries.json").read_text())
by_id = {e["id"]: e for e in entries}

for entry in entries:
    partners = entry.get("aspect_partners")
    assert isinstance(partners, list), f"aspect_partners is not an array: {entry['id']}"
    if entry.get("aspect") is not None or partners:
        assert entry["official"] and entry["pos"] == "verb", (
            "aspect metadata outside official verb", entry["id"], entry["title"]
        )
    for partner in partners:
        target = by_id.get(partner["id"])
        assert target is not None, ("missing partner page", entry["id"], partner)
        assert partner["title"] == target["title"], (
            "partner title differs from target page", entry["id"], partner, target["title"]
        )
        assert any(p["id"] == entry["id"] for p in target["aspect_partners"]), (
            "non-reciprocal partner", entry["id"], partner["id"]
        )

api = root / "api"
meta = json.loads((api / "meta.json").read_text())
lemmas = json.loads((api / "lemmas.json").read_text())
pairs = json.loads((api / "aspect-pairs.json").read_text())
assert meta["schema_version"] == 3
assert pairs["schema_version"] == 3 and len(pairs["pairs"]) == 1440
for row in lemmas["lemmas"]:
    assert len(row) == 8, ("lemma tuple width", len(row), row[:2])
    assert isinstance(row[7], list), ("aspect_partners is not an array", row[:2])
    is_official_verb = row[1] == "verb" and row[2] in ("official", "official-only")
    if is_official_verb:
        page = by_id.get(row[4])
        assert page is not None, ("official verb lemma missing page", row[:6])
        expected_partners = sorted(
            [[partner["id"], partner["title"]] for partner in page["aspect_partners"]]
        )
        assert row[6] == page["aspect"], ("lemma aspect differs from page", row, page)
        assert sorted(row[7]) == expected_partners, (
            "lemma partners differ from page", row, expected_partners
        )
    else:
        assert row[6] is None and row[7] == [], (
            "aspect metadata outside official verb lemma", row
        )

for pair in pairs["pairs"]:
    endpoints = {}
    for side in ("imperfective", "perfective"):
        endpoint = pair[side]
        entry_id = endpoint["entry_id"]
        assert entry_id in by_id, ("pair endpoint missing page", side, endpoint)
        endpoints[side] = by_id[entry_id]
    imperfective = endpoints["imperfective"]
    perfective = endpoints["perfective"]
    assert any(p["id"] == perfective["id"] for p in imperfective["aspect_partners"]), (
        "pair missing imperfective-to-perfective link", pair
    )
    assert any(p["id"] == imperfective["id"] for p in perfective["aspect_partners"]), (
        "pair missing perfective-to-imperfective link", pair
    )

# `total_bytes` is payload bytes and intentionally excludes meta.json itself.
counted = [api / "lemmas.json", api / "agent-guide.md", api / "router-selftest.json",
           api / "aspect-pairs.json"]
counted.extend((api / "forms").glob("*.json"))
actual_bytes = sum(path.stat().st_size for path in counted)
assert meta["total_bytes"] == actual_bytes, (
    "api total_bytes mismatch", meta["total_bytes"], actual_bytes
)
print(
    f"aspect export valid: {len(entries)} entries, "
    f"{sum(len(e['aspect_partners']) for e in entries)} directed links, "
    f"{len(pairs['pairs'])} model pairs, {len(lemmas['lemmas'])} API lemmas"
)
