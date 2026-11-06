# streamprobe task runner: `make help` lists the targets.

.DEFAULT_GOAL := help
.PHONY: help build test lint fmt run test-faults deny notices install

PREFIX ?= /usr/local
# Targets for `make notices` (default: this machine).
TARGETS ?= $(shell rustc -vV | sed -n 's/^host: //p')

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "  %-12s %s\n", $$1, $$2}'

build: ## Build the release binary (target/release/streamprobe)
	cargo build --release --locked

test: ## Unit tests (no network, no ffmpeg)
	cargo test --locked

lint: ## Check formatting, clippy and the shell scripts (no writes)
	cargo fmt --check
	cargo clippy --locked --all-targets -- -D warnings
	sh -n install.sh
	@if command -v shellcheck >/dev/null 2>&1; then shellcheck -s sh install.sh tests/*.sh; else echo "shellcheck not installed: skipped"; fi

fmt: ## Format the code
	cargo fmt

run: ## Probe a stream: make run URL=https://example.com/live
	cargo run --release -- "$(URL)"

test-faults: build ## End-to-end: fake Icecast with injected faults (needs ffmpeg + python3, ~2 min)
	sh tests/run_faults.sh

deny: ## Dependency policy: advisories, licences, bans, sources (needs cargo-deny)
	cargo deny --locked check

notices: ## Write THIRD_PARTY_LICENSES.txt (make notices TARGETS="x86_64-unknown-linux-musl ...")
	python3 scripts/third_party_licenses.py $(foreach t,$(TARGETS),--target $(t))

install: build ## Install into $(PREFIX)/bin (default /usr/local)
	install -m 0755 target/release/streamprobe "$(PREFIX)/bin/streamprobe"
