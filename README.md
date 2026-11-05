# Web Archiver

A self-hosted web archive: save rendered pages, their sub-resources, and a
screenshot, then replay them later from your own machine.

The server keeps the exact HTML it receives, builds a rewritten copy that points
at locally captured assets, and serves both versions with version history,
collections, tags, highlights, and a capture queue. It is designed to work with
a companion browser extension, but any client that can upload HTML can use it.

> The project is early-stage (`0.0.x`). The web UI and API may still change.

## Companion browser extension

Use the **Web Archiver** extension to capture the rendered page you are looking
at and send it to this server.

- **Source and releases** — <https://github.com/zennozenith/web-archiver-browser-extention>
- **Firefox** — install from [Firefox Add-ons](https://addons.mozilla.org/en-US/firefox/addon/web-archiver)
- **Chrome / Chromium** — load the extension from the
  [GitHub Releases](https://github.com/zennozenith/web-archiver-browser-extention/releases)
  page (download the Chrome build and load it as an unpacked extension)

The extension is a separate repository and is not required to use the server.

## Features

- **Personal link library** — save links with a title, description, thumbnail,
  collection, tags, and pin state.
- **Versioned captures** — every capture of a page is kept as a new version,
  with the original upload, rewritten HTML, and optional screenshot.
- **Offline replay** — archived HTML loads stylesheets, scripts, images, fonts,
  media, and other captured resources from local immutable URLs.
- **Reader view** — a calm detail page with text-size controls, format switching,
  and saved text highlights.
- **Collections and tags** — nested collections, global tags, pinned links,
  unorganized items, and client-side filtering.
- **Public collections** — opt a collection into a read-only share page.
- **Pending queue** — queue URLs in bulk, claim them from a worker, retry failed
  captures, and attach tags before the page exists.
- **Content-addressed storage** — files are keyed by BLAKE3 and deduplicated
  across pages and versions.
- **Compression** — Brotli (or gzip) on disk, with `Accept-Encoding` negotiation
  and on-the-fly transcoding for clients without Brotli.
- **HTTP caching** — representation-specific ETags, `304 Not Modified`, `Vary`,
  and long-lived immutable caching for content URLs.
- **One SQLite database** — pages, captures, collections, tags, queue, assets,
  and reference counts live in a single portable file.
- **JSON API and export** — automate archiving or export the library as JSON.
- **Garbage collection** — delete unreferenced content rows and orphaned blobs.
- **Safe replay** — archived HTML is served with a restrictive sandboxing
  Content-Security-Policy; captured scripts are stored but not executed.

## How it works

```text
browser extension / API client
        │
        │  1. POST /api/page              (URL, title, tags, thumbnail …)
        │  2. POST /api/page/{id}/capture (rendered HTML, screenshot, metadata)
        ▼
┌──────────────────────┐
│ capture pipeline     │
│                      │
│  • store exact HTML  │──────────────► content blob (original)
│  • find assets       │
│  • fetch HTML/CSS    │──────────────► content blobs (deduplicated)
│  • rewrite URLs      │──────────────► content blob (served HTML)
└──────────┬───────────┘
           │
           ▼
┌──────────────────────┐        ┌─────────────────────────┐
│ SQLite metadata      │        │ data/blobs/ab/cd/…      │
│ pages, captures,     │        │ compressed BLAKE3 blobs │
│ tags, collections    │        └────────────┬────────────┘
└──────────┬───────────┘                     │
           │                                 │
           ▼                                 ▼
  /api/page/{id}/latest/html          /api/content/{hash}
  /api/capture/{id}/html              /api/content/{hash}
```

A capture stores two HTML blobs: the **original** bytes exactly as uploaded, and
a **rewritten** copy where asset URLs point at `/api/content/{hash}` and relative
links are resolved against the original page URL. The rewritten copy is what the
replay endpoints serve.

Asset discovery and fetching are bounded: up to 100 resources per capture, 32 MiB
per resource, 128 MiB total, with CSS `@import` and `url(...)` references followed
up to four levels deep.

## Tech stack

| Layer     | Technology                                                                 |
| --------- | -------------------------------------------------------------------------- |
| Server    | Rust 2024, [Axum](https://github.com/tokio-rs/axum), Tokio                  |
| Database  | SQLite via [SQLx](https://github.com/launchbadge/sqlx), embedded migrations |
| Templates | [Askama](https://github.com/askama-rs/askama)                              |
| HTTP      | [reqwest](https://github.com/seanmonstar/reqwest) with rustls               |
| Storage   | BLAKE3 content addressing, Brotli/gzip, local blob store                   |
| Frontend  | Vite+ (Vite/Rolldown), TypeScript, Alpine.js, htmx, Tailwind CSS, DaisyUI  |
| Icons     | Iconify + Lucide, rendered as Tailwind classes                             |

Frontend details, conventions, and workflows live in
[`frontend/README.md`](frontend/README.md).

## Quick start

### With devenv (recommended)

[devenv](https://devenv.sh/) pins the Rust and frontend toolchains used by this
repository.

```bash
devenv shell

# Create data/db (once).
devenv tasks run init:server

# Build the frontend into static/vite.
devenv tasks run frontend:build

cargo run
```

Open <http://localhost:8889/dashboard>.

The devenv environment sets these defaults:

```text
APP_SERVER__HOST=0.0.0.0
APP_SERVER__PORT=8889
APP_DATABASE__URL=sqlite://data/db/app.sqlite3
APP_DATA_DIR=./data
OUT_DIR=../static/vite
```

`build.rs` runs the frontend build through devenv automatically, so a plain
`cargo run` from inside the devenv shell also works after the first setup.

### Manual setup

You need Rust (stable 1.85+ for the 2024 edition, or nightly), the Vite+ CLI
(`vp`), and Node.js. Install Vite+ with:

```bash
curl -fsSL https://vite.plus | bash
```

Then build the frontend into the directory Askama reads:

```bash
mkdir -p data/db

cd frontend
vp install
OUT_DIR=../static/vite vp build
cd ..

cargo run
```

`build.rs` currently invokes `devenv tasks run frontend` before compiling. If you
are not using devenv, either install it or adjust that invocation to run the
frontend build directly. Askama needs the generated templates under
`static/vite/src/templates` at compile time.

## Configuration

Settings are loaded from an optional `config/{APP_ENV}.toml` file and then from
environment variables. Environment variables use the `APP_` prefix, `__` as the
nested separator, and `_` after the prefix:

```bash
APP_SERVER__HOST=127.0.0.1 \
APP_SERVER__PORT=8889 \
APP_DATABASE__URL='sqlite://data/db/app.sqlite3' \
APP_DATA_DIR='./data' \
cargo run
```

| Variable                         | Default              | Description                                     |
| -------------------------------- | -------------------- | ----------------------------------------------- |
| `APP_ENV`                        | `local`              | Selects `config/{APP_ENV}.toml` when it exists  |
| `APP_DATA_DIR`                   | `data`               | Base directory for the blob store               |
| `APP_SERVER__HOST`               | `127.0.0.1`          | Listen address                                  |
| `APP_SERVER__PORT`               | *required*           | Listen port                                     |
| `APP_DATABASE__URL`              | `sqlite://data/db/app.db` | SQLite connection URL                      |
| `APP_DATABASE__MAX_CONNECTIONS`  | `10`                 | Connection pool size                            |
| `APP_DATABASE__BUSY_TIMEOUT_SEC` | `5`                  | SQLite busy timeout in seconds                  |
| `RUST_LOG`                       | `web_archiver=debug` | `tracing-subscriber` environment filter         |

The database directory must exist before startup; SQLite creates the database
file but not its parent directory. Blobs are stored under `APP_DATA_DIR/blobs`.
Migrations run automatically on startup.

## Web UI

The server-rendered UI is available at `/` (redirects to `/dashboard`).

| Path                          | Page                                              |
| ----------------------------- | ------------------------------------------------- |
| `/dashboard`                  | Library overview, stats, and recent links         |
| `/links`                      | All saved links, with infinite scroll and filters |
| `/links/{id}`                 | Reader/detail page with tags, highlights, formats |
| `/pinned`                     | Pinned links                                      |
| `/collections`                | Collection overview                               |
| `/collections/{id}`           | Links in a collection                             |
| `/collections/unorganized`    | Links with no collection                          |
| `/pending`                    | Capture queue                                     |
| `/tags`                       | All tags                                          |
| `/tags/{name}`                | Links with a tag                                  |
| `/public/collections/{id}`    | Read-only public collection                       |
| `/export`                     | JSON export of collections and links              |
| `/archive`                    | Compact archive dashboard (capture history)       |

Keyboard shortcuts: `Ctrl/Cmd+K` or `/` opens search.

## API overview

All JSON endpoints live under `/api`. The authoritative route list and request
details are documented at the top of [`src/api/mod.rs`](src/api/mod.rs).

| Group         | Endpoints                                                                                     |
| ------------- | --------------------------------------------------------------------------------------------- |
| Health/stats  | `GET /api/health`, `GET /api/stats`, `POST /api/maintenance/gc`                               |
| Collections   | `GET/POST /api/collection`, `GET/POST/PUT/DELETE /api/collection/{id}`, sharing, public read  |
| Pending       | `GET/POST/DELETE /api/pending`, bulk, claim, retry, fail, per-item get/patch/delete            |
| Pages         | `GET/POST /api/page`, lookup, `GET/PATCH/DELETE /api/page/{id}`, thumbnail, capture history    |
| Captures      | `POST /api/page/{id}/capture`, `GET/DELETE /api/capture/{id}`, HTML/original/screenshot/assets |
| Content       | `GET/HEAD /api/content/{hash}`                                                                |
| Tags          | `GET/POST /api/tag`, `PATCH/DELETE /api/tag/{id}`, `POST /api/tag/{id}/merge`                 |

`slumber.yml` contains an interactive request collection for exploring the API
with [Slumber](https://slumber.lucaspickering.me/).

### Archive a page with curl

Archiving is two requests: create (or look up) the page, then attach a capture.

```bash
# 1. Create the page. `url` and `displayTitle` are required.
curl -X POST http://localhost:8889/api/page \
  -F 'url=https://example.com/article' \
  -F 'displayTitle=Example article' \
  -F 'tags=reading,example'

# The response contains the page id, e.g. "id": 1.

# 2. Upload a rendered capture. `html` is required.
curl -X POST http://localhost:8889/api/page/1/capture \
  -F 'html=@page.html;type=text/html' \
  -F 'title=Example article' \
  -F 'saveAssets=true'
```

Useful capture fields:

| Field         | Required | Description                                                |
| ------------- | -------- | ---------------------------------------------------------- |
| `html`        | yes      | Rendered HTML to archive                                   |
| `title`       | no       | Capture title; the metadata `title` is a fallback          |
| `screenshot`  | no       | Screenshot bytes (PNG or JPEG; detected by magic bytes)    |
| `metadata`    | no       | JSON object supplied by the client                         |
| `pendingId`   | no       | Claim a queued item, merge its tags, and dequeue it        |
| `saveAssets`  | no       | Fetch and store the document's sub-resources (default off) |

A successful capture returns `201 Created` with `captureId`, `pageId`, `version`,
deduplication flags, and an asset summary (`saved`, `deduplicated`, `failed`,
`skipped`). Check whether a URL is already archived with
`GET /api/page/lookup?url=...`.

### Replay endpoints

| Endpoint                                    | Returns                                    |
| ------------------------------------------- | ------------------------------------------ |
| `/api/capture/{id}/html`                    | Rewritten HTML that loads captured assets  |
| `/api/capture/{id}/html/original`           | Exact bytes uploaded for that capture      |
| `/api/capture/{id}/screenshot`              | Stored screenshot, if any                  |
| `/api/capture/{id}/asset`                   | Assets saved with the capture              |
| `/api/page/{id}/latest/html`                | Redirect to the newest rewritten capture   |
| `/api/page/{id}/latest/html/original`       | Redirect to the newest original upload     |
| `/api/page/{id}/latest/screenshot`          | Redirect to the newest screenshot          |
| `/api/content/{hash}`                       | Immutable deduplicated blob                |

Archived HTML is sandboxed: scripts are captured as assets but are **not**
executed on replay.

### Pending queue workflow

A capture worker can avoid holding the page in memory while the user browses:

```text
POST /api/pending          queue a URL (optionally with tags)
POST /api/pending/claim    atomically take the next queued item
POST /api/page             create the page when the capture is ready
POST /api/page/{id}/capture with pendingId → merges tags and dequeues
```

Failed items can be retried with `POST /api/pending/{id}/retry` or marked with
`POST /api/pending/{id}/fail`.

## Storage and maintenance

- **Pages** are canonicalized URLs with editable metadata.
- **Captures** are immutable versions of a page.
- **Content** rows are unique blobs keyed by BLAKE3, optionally compressed and
  either inline in SQLite or on disk under `data/blobs`.
- **Capture assets** link a capture to blobs by their original URL and kind.
- **Tags** are global and shared by pages and pending items.
- **Reference counts** are maintained by SQLite triggers.

Deleting a page or capture does not immediately remove its blobs. Run garbage
collection to remove unreferenced content older than one hour and clean up
abandoned temporary files:

```bash
curl -X POST http://localhost:8889/api/maintenance/gc
```

## Development

```bash
# Rust
cargo fmt
cargo clippy --all-targets
cargo test

# Frontend
cd frontend
vp lint
vp fmt --check
OUT_DIR=../static/vite vp build
```

Useful source areas:

```text
src/
  api/         JSON API handlers and route definitions
  web/         server-rendered library UI and HTMX handlers
  web_archive/ compact archive dashboard (capture history)
  db/          SQLite queries, blob metadata, and garbage collection
  extractor/   HTML/CSS resource discovery and URL rewriting
  downloader.rs concurrent sub-resource fetching
  serve.rs     blob serving, compression negotiation, ETag/304 handling
  main.rs      application setup and routing
frontend/
  src/         TypeScript, CSS, and Askama source templates
migrations/    SQLite schema
static/vite/   generated frontend output (ignored by git)
```

Frontend source templates are built by Vite into `static/vite/src/templates`,
which is the directory Askama compiles against. See
[`frontend/README.md`](frontend/README.md) for the frontend architecture,
template rules, and contribution workflow.

## Security notes

This is a single-user, self-hosted application. Bearer-token middleware exists in
`src/api/mod.rs` but is **not enabled** in the default router, and the CSRF token
in the UI is a placeholder. Do not expose the server directly to the public
internet without authentication, a reverse proxy, and network controls.

Replay is sandboxed with a restrictive Content-Security-Policy, but the server
does fetch sub-resources from URLs referenced by uploaded documents. Run it on a
trusted network or add egress filtering if you accept captures from untrusted
clients.

## License

MIT. See [LICENSE](LICENSE).
