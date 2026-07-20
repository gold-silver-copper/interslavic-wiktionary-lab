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
assert meta["schema_version"] == 4
assert pairs["schema_version"] == 3 and len(pairs["pairs"]) == 1440
for row in lemmas["lemmas"]:
    # Schema 4: [lemma, pos, status, probability, entry_id, gloss, aspect,
    #            aspect_partners, frequency, langs, branch_pattern, borrowed].
    assert len(row) == 12, ("lemma tuple width", len(row), row[:2])
    assert isinstance(row[7], list), ("aspect_partners is not an array", row[:2])
    assert row[8] is None or isinstance(row[8], (int, float)), ("frequency type", row[:2], row[8])
    assert isinstance(row[9], int), ("langs type", row[:2], row[9])
    assert row[10] is None or isinstance(row[10], str), ("branch_pattern type", row[:2], row[10])
    assert isinstance(row[11], bool), ("borrowed type", row[:2], row[11])
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

# English lookup selftest: reimplement the normalization + router independently
# of the Rust exporter, so drift between the published samples and the actual
# contract fails here instead of in a client.
def en_normalize(raw):
    folded = "".join(ch if ch.isalnum() else " " for ch in raw.lower())
    key = " ".join(folded.split())
    if key.startswith("to ") and key[3:].strip():
        key = key[3:].strip()
    return key

def fnv1a32(text):
    h = 2166136261
    for b in text.encode("utf-8"):
        h = ((h ^ b) * 16777619) & 0xFFFFFFFF
    return h

def en_desuffix(key):
    """Independent reimplementation of the documented de-suffix ladder."""
    out = []

    def push(cand):
        if len(cand) >= 3 and cand != key and cand not in out:
            out.append(cand)

    rules = [
        ("ibility", ["ible"]), ("ability", ["able"]), ("iness", ["y"]),
        ("ness", [""]), ("ation", ["", "ate"]), ("ition", ["", "e", "ite"]),
        ("ity", ["", "e"]), ("ing", ["", "e"]), ("ies", ["y"]),
        ("es", [""]), ("s", [""]),
    ]
    for suf, restores in rules:
        if key.endswith(suf):
            stem = key[: len(key) - len(suf)]
            for r in restores:
                push(stem + r)
            if (suf == "ing" and len(stem) >= 2 and stem[-1] == stem[-2]
                    and stem[-1].isascii() and stem[-1].isalpha()):
                push(stem[:-1])
    return out

en_selftest = json.loads((api / "en" / "selftest.json").read_text())
assert en_selftest["samples"], "en selftest has no samples"
for raw, key, shard in en_selftest["samples"]:
    assert en_normalize(raw) == key, ("en normalization drift", raw, key, en_normalize(raw))
    assert fnv1a32(key) % en_selftest["shards"] == shard, ("en router drift", key, shard)
assert en_selftest["desuffix_samples"], "en selftest has no desuffix samples"
for key, variants in en_selftest["desuffix_samples"]:
    assert en_desuffix(key) == variants, (
        "en de-suffix ladder drift", key, variants, en_desuffix(key)
    )

# `total_bytes` is payload bytes and intentionally excludes meta.json itself.
counted = [api / "lemmas.json", api / "agent-guide.md", api / "router-selftest.json",
           api / "aspect-pairs.json", api / "suggest-selftest.json", api / "notes.json"]
counted.extend((api / "forms").glob("*.json"))
counted.extend((api / "suggest").glob("*.json"))
counted.extend((api / "en").glob("*.json"))
actual_bytes = sum(path.stat().st_size for path in counted)
assert meta["total_bytes"] == actual_bytes, (
    "api total_bytes mismatch", meta["total_bytes"], actual_bytes
)
print(
    f"aspect export valid: {len(entries)} entries, "
    f"{sum(len(e['aspect_partners']) for e in entries)} directed links, "
    f"{len(pairs['pairs'])} model pairs, {len(lemmas['lemmas'])} API lemmas"
)
