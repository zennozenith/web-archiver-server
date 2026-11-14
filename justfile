# Default settings
set shell := ["bash", "-uc"]
set dotenv-load := false
set default-list

# export RUST_LOG := "info,myopic=debug"
# export OUT_DIR := "../static/vite"
# export APP_SERVER__HOST := "0.0.0.0"
# export APP_SERVER__PORT := "8889"
# export APP_PUBLIC_URL := "http://localhost:8889"
# export APP_DATABASE__URL := "sqlite://data/db/app.sqlite3"
# export APP_DATA_DIR := "./data"
# export DATABASE_URL := "sqlite://data/db/app.sqlite3"

[working-directory: 'frontend/public/vendored']
vendored-compress:
    #!/usr/bin/env bash
    set -euo pipefail
    for file in *; do
        # Skip directories
        [ -d "$file" ] && continue

        case "$file" in
            *.br)
                # Skip files already ending with .br
                ;;
            *)
                echo "Processing: $file"
                ouch compress "$file" "$file.br"
                ;;
        esac
    done

# Run cargo tests
app-test:
    cargo test

# Initialize the server data directory
init-server:
    mkdir -p data/db && \
    sqlx database create && \
    sqlx migrate run

# Install frontend dependencies
[working-directory: 'frontend']
init-frontend:
    vp install

# Build the frontend (lint, format, build)
[working-directory: 'frontend']
frontend-build:
    vpr lint && vpr fmt && vpr build

# frontend-tailwind:
#     bunx @tailwindcss/cli -i ./frontend/input.css -o ./static/css/build/tailwind.css


# Build frontend then run the server
run: frontend-build
    cargo run

# Watch src/ and frontend/ and re-run the server
watcher:
    watchexec -q -c -w src/ -w frontend/ -r --stop-signal SIGKILL "cargo run"

# Seed the database (requires gum, fish, sqlite)
seed-db:
    fish ./seed-db.fish

# Download vendored frontend assets and compress them
vendor-download:
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf frontend/public/vendored
    mkdir -p frontend/public/vendored
    cd frontend/public/vendored

    wget -O htmx-4.0.0.js      https://cdn.jsdelivr.net/npm/htmx.org@4.0.0/dist/htmx.js
    wget -O htmx-4.0.0.min.js  https://cdn.jsdelivr.net/npm/htmx.org@4.0.0/dist/htmx.min.js
    wget -O htmax-4.0.0.js     https://cdn.jsdelivr.net/npm/htmx.org@4.0.0/dist/htmax.js
    wget -O htmax-4.0.0.min.js https://cdn.jsdelivr.net/npm/htmx.org@4.0.0/dist/htmax.min.js

    just vendored-compress

# Watch tailwind inputs and rebuild (task is commented out upstream, but kept for parity)
watch-tailwind:
    watchexec -e html,css -w frontend/input.css -w templates/ -- just frontend-tailwind
