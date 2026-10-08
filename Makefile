VERSION := $(shell awk -F\" '/^version = / { print $$2; exit }' Cargo.toml)

RELEASE_BRANCH := master

# Example project used by the try targets: hello-c or hello-rust.
EXAMPLE ?= hello-c
BSDT := $(CURDIR)/target/debug/bsdt

.DEFAULT_GOAL := help

.PHONY: help build install test lint docs try try-down try-destroy publish-check publish

help: ## Show this help
	@echo "Usage: make <target>"
	@echo
	@awk 'BEGIN { FS = ":.*## " } /^[a-z-]+:.*## / { printf "  \033[1m%-14s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

build: ## Build the release binary (target/release/bsdt)
	cargo build --release

install: ## Install the bsdt binary and its man page into ~/.cargo from this checkout
	cargo install --locked --path .
	bsdt man --install

test: ## Run all tests
	cargo test

lint: ## Run clippy and lint the man page
	cargo clippy --all-targets
	@command -v mandoc >/dev/null && mandoc -T lint -W warning man/bsdt.1 || true

try: ## Debug-build bsdt and boot an example VM (EXAMPLE=hello-c, the default, hello-rust or gui)
	cargo build
	cd examples/$(EXAMPLE) && $(BSDT) up
	@echo
	@echo "Now: export PATH=\"$(CURDIR)/target/debug:\$$PATH\"; cd examples/$(EXAMPLE)"
	@echo "and use bsdt exec/ssh/sync there; see TESTING.md."

try-down: ## Shut down the example VM, keeping its disk
	cd examples/$(EXAMPLE) && $(BSDT) down

try-destroy: ## Delete the example VM so the next try starts fresh
	cd examples/$(EXAMPLE) && $(BSDT) destroy

docs: docs/index.html ## Render the man page to docs/index.html (needs mandoc)

docs/index.html: man/bsdt.1
	@command -v mandoc >/dev/null || { echo "mandoc not found — install it (e.g. apt install mandoc)"; exit 1; }
	mandoc -T lint -W warning $<
	mandoc -T html -O 'style=style.css,man=https://man.freebsd.org/cgi/man.cgi?query=%N&sektion=%S' $< > $@

publish-check: ## Verify the crate can be published (on master, clean, pushed, dry run passes)
	@branch="$$(git rev-parse --abbrev-ref HEAD)"; \
		test "$$branch" = "$(RELEASE_BRANCH)" || { echo "publishing is done from $(RELEASE_BRANCH), but you are on $$branch"; exit 1; }
	@test -z "$$(git status --porcelain)" || { echo "working tree has uncommitted changes"; exit 1; }
	@git fetch --quiet origin "$(RELEASE_BRANCH)"
	@test "$$(git rev-parse HEAD)" = "$$(git rev-parse "origin/$(RELEASE_BRANCH)")" || { echo "HEAD differs from origin/$(RELEASE_BRANCH) — push or pull first"; exit 1; }
	cargo test
	cargo publish --dry-run
	@echo "Ready to publish bsdt $(VERSION)"

publish: publish-check ## Publish bsdt to crates.io
	cargo publish
