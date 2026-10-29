# web-archiver

A self-hosted web archive server: save rendered pages, their sub-resources, and
replay them later from your own machine.

The server stores the exact HTML it receives, builds a rewritten copy that points
at locally captured assets, and serves both versions with version history, tags,
and a pending capture queue. It is designed to work with a companion browser
extension, but any client that can `POST` HTML can use it.

## Features

- **Versioned page archive** — every capture belongs to a page and gets a version
  number; old versions remain available.
- **Original + rewritten HTML** — the uploaded bytes are preserved exactly, while
  the served copy points resources at immutable local blob URLs.
- **Sub-resource capture** — stylesheets, scripts, images, fonts, media, frames,
  icons, and manifests referenced by the document are fetched and stored.
- **CSS rewriting** — `url(...)` and `@import` references are captured and
  rewritten recursively (bounded depth).
- **Links stay useful** — anchor links are absolutized to the original site;
  fragment-only links stay in-page.
- **Content-addressed storage** — blobs are keyed by BLAKE3 and deduplicated
  across every page and version.
- **Compression** — blobs are stored as Brotli (or gzip) and served with
  `Accept-Encoding` negotiation, including on-the-fly gzip transcoding for
  clients that do not advertise Brotli.
- **HTTP caching** — representation-specific ETags, `304 Not Modified`, `Vary`,
  and long-lived immutable caching for content URLs.
- **SQLite metadata** — pages, captures, tags, pending queue, asset links, and
  reference counts live in one portable database.
- **Web UI** — server-rendered Askama pages with HTMX, Alpine.js, Tailwind CSS,
  and DaisyUI.
- **JSON API** — archive, browse, tag, queue, and maintain the archive from any
  client.
- **Garbage collection** — removes unreferenced content rows and orphaned blob
  files.
- **Safety defaults** — archived HTML is served with a restrictive
  `Content-Security-Policy: sandbox` that disables scripts and framing.

## How it works

```text
client / browser extension
        │  POST /api/archive (HTML + optional screenshot + metadata)
        ▼
  ┌───────────────┐
  │ archive API   │
  └───────┬───────┘
          │ 1. store exact uploaded HTML (original blob)
          │ 2. discover resources in HTML/CSS
          │ 3. fetch and store assets as content blobs
          │ 4. rewrite HTML: assets → /api/content/{hash}
          │                  links  → original site URLs
          ▼
  ┌───────────────┐        ┌────────────────────┐
  │ SQLite        │        │ blob store          │
  │ metadata      │        │ data/blobs/...      │
  └───────┬───────┘        └─────────┬──────────┘
          │                          │
          ▼                          ▼
  GET /api/captures/{id}/html   GET /api/content/{hash}
  GET /api/pages/{id}/latest/html
```

The uploaded document and the rewritten document are separate blobs. A capture's
`content_id` points at the rewritten HTML; `original_content_id` points at the
uploaded bytes.

## Quick start

### Prerequisites

- [devenv](https://devenv.sh/) (recommended; the repository pins the Rust and
  frontend toolchain)
- Rust (the `devenv.nix` environment uses nightly)
- Vite+ / `vp` and the Node toolchain provided by devenv for the `frontend/` app

### Run with devenv

```bash
devenv shell

# Build the frontend once (or let cargo do it through build.rs).
devenv tasks run frontend:build

cargo run
```

The devenv environment defaults to:

```text
http://0.0.0.0:8889
sqlite://data/db/app.sqlite3
```

Open `http://localhost:8889/` for the web UI.

### Build without the devenv task runner

The frontend build is required before compiling, because Askama reads the
generated templates from `static/vite/src/templates`:

```bash
cd frontend
vp install       # first time only
vp run build
cd ..
```

`build.rs` normally invokes `devenv tasks run frontend` automatically. If you are
not using devenv, run the frontend build first and adjust that step in
`build.rs` for your environment.

### Configuration

Settings are loaded from an optional `config/{APP_ENV}.toml` file and then from
environment variables. Environment variables use the `APP_` prefix, `__` as the
nested separator, and `_` after the prefix:

```bash
APP_SERVER__HOST=127.0.0.1 \
APP_SERVER__PORT=8889 \
APP_DATABASE__URL='sqlite://data/db/app.sqlite3' \
cargo run
```

| Variable | Default | Description |
| --- | --- | --- |
| `APP_ENV` | `local` | Selects `config/{APP_ENV}.toml` when present |
| `APP_SERVER__HOST` | `127.0.0.1` | Listen address |
| `APP_SERVER__PORT` | required | Listen port |
| `APP_DATABASE__URL` | `sqlite://db/app.db` | SQLite connection URL |
| `APP_DATABASE__MAX_CONNECTIONS` | `10` | Connection pool size |
| `APP_DATABASE__BUSY_TIMEOUT_SEC` | `5` | SQLite busy timeout |

Blobs are stored under `data/blobs`. Migrations run automatically on startup.

## Using the web UI

The index page (`/`) provides:

- **Archived pages** — newest first, with title, URL, last capture time, and
  version count. Expand a page to view and open each captured version.
- **Capture replay** — open the rewritten HTML, the exact original upload, or the
  screenshot.
- **Tags** — add or remove tags per page; create, rename, and delete tags in the
  sidebar.
- **Pending queue** — add URLs (with optional title and tags), or remove items
  that have not been captured yet.
- **Filtering** — filter archived pages by title, URL, or tag without a reload.

Archiving is intentionally not part of the index page; captures are created by
the extension or the archive API.

## Archiving a page

`POST /api/archive` accepts `multipart/form-data`:

```bash
curl -X POST http://localhost:8889/api/archive \
  -F 'html=@page.html;type=text/html' \
  -F 'url=https://example.com/article' \
  -F 'title=Example article' \
  -F 'tags=["reading","example"]' \
  -F 'save-assets=true'
```

Fields:

| Field | Required | Description |
| --- | --- | --- |
| `html` | yes | Rendered HTML to archive |
| `url` | yes* | Original page URL (*can come from metadata or a pending item) |
| `title` | no | Page title; metadata and the pending item are fallbacks |
| `tags` | no | JSON array of tag names |
| `pending_id` | no | Claim a queued URL and use its URL/title |
| `screenshot` | no | Screenshot bytes |
| `screenshot-format` | no | `png` or `jpeg` when a screenshot is sent |
| `metadata` | no | JSON object; `url` and `title` are recognized |
| `save-assets` | no | `true` (default) fetches sub-resources |

Example response (`201 Created`):

```json
{
  "captureId": 12,
  "pageId": 4,
  "version": 3,
  "deduplicated": {
    "html": false,
    "originalHtml": false,
    "screenshot": null
  },
  "assets": {
    "saved": 18,
    "deduplicated": 4,
    "failed": 0,
    "skipped": 0
  }
}
```

## Replaying a capture

| Endpoint | Returns |
| --- | --- |
| `/api/captures/{id}/html` | Rewritten HTML that loads captured assets |
| `/api/captures/{id}/html/original` | Exact bytes uploaded to the archive API |
| `/api/captures/{id}/screenshot` | Stored screenshot, if any |
| `/api/pages/{id}/latest/html` | Redirect to the newest rewritten capture |
| `/api/pages/{id}/latest/html/original` | Redirect to the newest original capture |
| `/api/content/{hash}` | Immutable deduplicated blob |

Archived HTML is sandboxed. Scripts are captured as assets but are **not**
executed on replay; this keeps archived pages from running untrusted code on your
origin.

## API overview

The complete route list and request details live in
[`src/api/mod.rs`](src/api/mod.rs). Main groups:

| Group | Endpoints |
| --- | --- |
| Archive | `POST /api/archive` |
| Pending queue | `GET/POST /api/pending`, `PATCH/DELETE /api/pending/{id}`, bulk/claim/retry/fail |
| Pages | `GET /api/pages`, `GET/DELETE /api/pages/{id}`, captures, latest HTML |
| Captures | `GET/DELETE /api/captures/{id}`, HTML/original/screenshot, assets |
| Content | `GET/HEAD /api/content/{hash}` |
| Tags | `GET/POST /api/tags`, `PATCH/DELETE /api/tags/{id}`, merge |
| Maintenance | `POST /api/maintenance/gc` |
| Misc | `GET /api/health`, `GET /api/stats` |

`slumber.yml` contains an interactive request collection for exploring the API
with [Slumber](https://slumber.lucaspickering.me/).

### Authentication

A Bearer-token middleware exists in `src/api/mod.rs`, but it is not enabled in the
default router. Do not expose the API directly to the public internet without
putting it behind authentication (a reverse proxy or the provided middleware).

## Development

```bash
# Rust
cargo fmt
cargo test

# Frontend
cd frontend
vp run build
```

Useful source areas:

```text
src/
  api/        JSON API handlers and route definitions
  web/        server-rendered UI, HTMX handlers, Askama templates
  db/         SQLite queries, content-addressed blob storage, GC helpers
  extractor/  HTML/CSS resource discovery and URL rewriting
  serve.rs    blob serving, compression negotiation, ETag/304 handling
  main.rs     application setup and routing
frontend/
  src/        TypeScript, CSS, and Askama source templates
migrations/    SQLite schema
```

Frontend source templates are built by Vite into `static/vite/src/templates`,
which is what Askama compiles against. `static/vite` is generated and ignored by
version control.

## Storage and maintenance

- **Pages** are canonicalized URLs (`url`).
- **Captures** are immutable versions of a page (`capture`).
- **Content** rows are unique blobs keyed by BLAKE3 (`content`).
- **Assets** link a capture to blobs by original URL (`capture_asset`).
- **Tags** are global, with links for pages (`url_tag`) and pending items
  (`pending_tag`).

Reference counts are maintained by SQLite triggers. Deleting a page or capture
does not delete content immediately; run the maintenance endpoint to collect
unreferenced blobs:

```bash
curl -X POST http://localhost:8889/api/maintenance/gc
```

## License

MIT. See [LICENSE](LICENSE).
