DUMP ?= /Users/kisaczka/Desktop/code/wikidata/raw-wiktextract-data.jsonl
WIKI_DIR ?= /Users/kisaczka/Desktop/code/wikidata
OFFICIAL ?= data/official-isv.csv
LEMMAS ?= data/slavic-lemmas.cache.json
SITE ?= site
OUT ?= target/eval

.PHONY: extract-proto extract-lemmas extract-raw-slavic extract-enrich extract-all \
	eval proto-eval corpus-eval aspect-eval audit export serve explain coverage check fmt test clean \
	search-perf

# One-time: stream the 23GB dump into the Proto-Slavic cache (enables +proto-derived).
extract-proto:
	cargo run --release -- extract-proto --dump "$(DUMP)"

# One-time: stream the dump into the Slavic-lemma corpus (drives the cognate-set site).
extract-lemmas:
	cargo run --release -- extract-lemmas --dump "$(DUMP)" --out "$(LEMMAS)"

# One-time: stream the dump into the RAW (evidence-free) Slavic lemma cache +
# extraction tally (drives the site's raw-attestation pages; issue #33/#34).
extract-raw-slavic:
	cargo run --release -- extract-raw-slavic --dump "$(DUMP)"

# Native RU/PL/CS Wiktionary enrichment. Needs the lemma + raw caches first
# (build_wanted unions both), so run it AFTER extract-lemmas/extract-raw-slavic.
extract-enrich:
	cargo run --release -- extract-enrich --dir "$(WIKI_DIR)"

# Rebuild every cache in dependency order after an extractor-logic change.
extract-all: extract-proto extract-lemmas extract-raw-slavic extract-enrich

# Raw-lemma coverage report (reconciles extraction tally vs export dedup).
coverage:
	cargo run --release -- coverage --out "$(OUT)"

# Reproducible accuracy benchmark against the official Interslavic dictionary.
eval:
	cargo run --release -- evaluate --official "$(OFFICIAL)" --out "$(OUT)"

# Proto-engine-only benchmark; data-quality/ceiling audit.
proto-eval:
	cargo run --release -- proto-eval --out "$(OUT)"
audit:
	cargo run --release -- audit --out "$(OUT)"

# The site path (corpus::generate_set) accuracy vs the official dictionary.
corpus-eval:
	cargo run --release -- corpus-eval

# Dedicated perfective↔imperfective pair benchmark (issue #75).
aspect-eval:
	cargo run --release -- aspect-eval --official "$(OFFICIAL)" --out "$(OUT)"

# Generate the static website locally (no server; not published anywhere).
export:
	cargo run --release -- export --lemmas "$(LEMMAS)" --official "$(OFFICIAL)" --out "$(SITE)"

# Preview the generated static site locally (any static server works).
serve: export
	cd "$(SITE)" && python3 -m http.server 8765

# Cold-search cost of the exported site, measured by driving the production
# search client under Node (tools/search-perf.mjs; issue #71).
search-perf:
	node tools/search-perf.mjs "$(SITE)" --out "$(OUT)/search-performance.md" --label "$$(git rev-parse --short HEAD)"

# Spot-check one word/gloss with a full rule trace, e.g. `make explain W=duša`.
explain:
	cargo run -- explain "$(W)"

fmt:
	cargo fmt
check:
	cargo check
test:
	cargo test
clean:
	rm -rf "$(SITE)" data/*.tmp
