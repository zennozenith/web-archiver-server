# Primary task runner. Run `just` to list recipes.
# Environment (env vars, toolchain, packages) comes from devenv.nix,
# so run inside the devenv shell (direnv or `devenv shell`).

# export APP_SERVER__PORT := "8889"

set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

htmx_version := "4.0.0"
vendored_dir := "frontend/public/vendored"

alias t := test
alias r := run
alias d := dev

# List all recipes
default:
    @just --list --unsorted

# ─── Setup ───────────────────────────────────────────────────────────

# One-shot first-time setup
[group('setup')]
setup: db-setup frontend-install

# Install the Vite+ CLI globally (not packaged in nix yet)
[group('setup')]
tool-install-vp:
    curl -fsSL https://vite.plus | bash

# ─── Run ─────────────────────────────────────────────────────────────

# Build the frontend, then run the server
[group('run')]
run *args:
    WEB_ARCHIVER_BUILD_FRONTEND=0 cargo run {{ args }}
# run *args: frontend-build
#     WEB_ARCHIVER_BUILD_FRONTEND=0 cargo run {{ args }}

# Rebuild + restart the server on changes to src/ and frontend/
[group('run')]
dev:
    watchexec -q -c -r -w src/ -w frontend/ --stop-signal SIGKILL -- cargo run

# Release build (frontend included)
[group('run')]
build: frontend-build
    WEB_ARCHIVER_BUILD_FRONTEND=0 cargo build --release

# ─── Quality ─────────────────────────────────────────────────────────

# Run tests (nextest + doctests)
[group('quality')]
test *args:
    cargo nextest run {{ args }}
    cargo test --doc

# Format Rust and frontend code
[group('quality')]
fmt:
    cargo fmt --all
    cd frontend && vpr fmt

# Clippy with warnings as errors
[group('quality')]
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Fast checks: formatting (no writes) + clippy
[group('quality')]
check:
    cargo fmt --all --check
    just clippy

# Check the layer direction encoded in the design document
[group('quality')]
layers:
    ./scripts/check-layers.sh

# Dependency vulnerabilities (cargo install cargo-audit)
[group('quality')]
audit:
    cargo audit

# Licenses, advisories, bans, sources (cargo install cargo-deny)
[group('quality')]
deny:
    cargo deny check

# All dependency checks
[group('quality')]
security: audit deny

# Everything CI should run
[group('quality')]
ci: check test security layers frontend-check frontend-build

# ─── Database ────────────────────────────────────────────────────────

# Create the database and apply migrations
[group('db')]
db-setup:
    mkdir -p data/db
    sqlx database create
    sqlx migrate run

# Apply pending migrations
[group('db')]
db-migrate:
    sqlx migrate run

# Drop, recreate and migrate the database
[confirm('This deletes the local database. Continue?')]
[group('db')]
db-reset:
    sqlx database drop -y
    just db-setup

# Seed the database (needs gum, fish, sqlite)
[group('db')]
db-seed:
    fish ./seed-db.fish

# ─── Frontend ────────────────────────────────────────────────────────

# Install frontend dependencies
[group('frontend')]
[working-directory: 'frontend']
frontend-install:
    vp install

# Lint and format-check the frontend
[group('frontend')]
[working-directory: 'frontend']
frontend-check:
    vpr lint
    vpr fmt

# Production frontend build (also invoked by build.rs)
[group('frontend')]
[working-directory: 'frontend']
frontend-build:
    vpr build

# ─── Vendored assets ─────────────────────────────────────────────────

# Download vendored htmx assets and brotli-compress them
[group('vendor')]
vendor-fetch:
    rm -rf {{ vendored_dir }}
    mkdir -p {{ vendored_dir }}
    cd {{ vendored_dir }} && \
    for name in htmx htmax; do \
        for ext in js min.js; do \
            wget -q -O "$name-{{ htmx_version }}.$ext" \
                "https://cdn.jsdelivr.net/npm/htmx.org@{{ htmx_version }}/dist/$name.$ext"; \
        done; \
    done
    just vendor-compress

# Brotli-compress every non-.br file in the vendored directory
[group('vendor')]
[working-directory: 'frontend/public/vendored']
vendor-compress:
    #!/usr/bin/env bash
    set -euo pipefail
    find . -maxdepth 1 -type f ! -name '*.br' -print0 |
    while IFS= read -r -d '' file; do
        echo "Compressing: ${file#./}"
        ouch compress --yes "$file" "$file.br"
    done

