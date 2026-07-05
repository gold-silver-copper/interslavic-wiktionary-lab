DUMP ?= /Users/kisaczka/Desktop/code/english/raw-wiktextract-data.jsonl
OFFICIAL ?= data/official-isv.csv
SITE ?= site
OUT ?= target/eval

.PHONY: extract-proto eval proto-eval audit export serve explain check fmt test clean

# One-time: stream the 23GB dump into the Proto-Slavic cache (enables +proto-derived).
extract-proto:
	cargo run --release -- extract-proto --dump "$(DUMP)"

# Reproducible accuracy benchmark against the official Interslavic dictionary.
eval:
	cargo run --release -- evaluate --official "$(OFFICIAL)" --out "$(OUT)"

# Proto-engine-only benchmark; data-quality/ceiling audit.
proto-eval:
	cargo run --release -- proto-eval --out "$(OUT)"
audit:
	cargo run --release -- audit --out "$(OUT)"

# Generate the static website (no server, GitHub Pages hostable).
export:
	cargo run --release -- export --out "$(SITE)"

# Preview the generated static site locally (any static server works).
serve: export
	cd "$(SITE)" && python3 -m http.server 8765

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
