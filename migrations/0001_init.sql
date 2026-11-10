-- =============================================================================
-- Web archive schema (SQLite >= 3.38: STRICT tables, unixepoch())
-- Schema version: 1  (tracked via PRAGMA user_version)
--
-- Conventions
--   * All timestamps are unix epoch seconds (UTC), INTEGER.
--   * All tables are STRICT: column types are enforced, not advisory.
--   * Every foreign key declares its ON DELETE behaviour explicitly.
--   * Every foreign key column that can be hit by a parent DELETE is indexed,
--     so cascades and the content GC never fall back to full table scans.
--   * Booleans are INTEGER 0/1 with a CHECK.
--   * JSON columns are validated with json_valid().
--
-- Connection setup (per connection, NOT persisted in the file; do this in the
-- application's pool "after_connect" hook):
--     PRAGMA foreign_keys  = ON;
--     PRAGMA busy_timeout  = 5000;
--     PRAGMA synchronous   = NORMAL;      -- safe with WAL
--     PRAGMA temp_store    = MEMORY;
-- =============================================================================

-- PRAGMA foreign_keys = ON;       -- no-op inside a transaction, so set before BEGIN
-- PRAGMA journal_mode = WAL;      -- persistent; cannot be changed inside a transaction

-- BEGIN;

-- -----------------------------------------------------------------------------
-- tag: global, shared across all pages
-- -----------------------------------------------------------------------------
CREATE TABLE tag (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL COLLATE NOCASE,   -- 'Recipes' and 'recipes' are the same tag
    color       TEXT,                              -- palette key, e.g. 'red'; NULL = frontend decides
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    CONSTRAINT uq_tag_name        UNIQUE (name),
    CONSTRAINT ck_tag_name_nonblank CHECK (length(name) > 0 AND name = trim(name))
) STRICT;

-- -----------------------------------------------------------------------------
-- compression: lookup table; adding an algorithm is just an INSERT.
-- Ids are part of the on-disk contract with the application: never renumber.
-- -----------------------------------------------------------------------------
CREATE TABLE compression (
    id    INTEGER PRIMARY KEY,
    name  TEXT NOT NULL UNIQUE                     -- 'none', 'gzip', 'brotli', 'zstd', ...
) STRICT;

INSERT INTO compression (id, name) VALUES (0, 'none'), (1, 'gzip'), (2, 'brotli');

-- -----------------------------------------------------------------------------
-- content: deduplicated blobs, one row per unique file.
--
-- ref_count is the number of references from capture.content_id,
-- capture.original_content_id, capture.screenshot_content_id,
-- capture_asset.content_id and page.thumbnail_content_id. A blob referenced
-- twice by the same row (e.g. content_id = original_content_id) counts twice.
-- It is maintained exclusively by the triggers below; the application must
-- never write it. See v_content_ref_drift for an integrity audit.
--
-- GC contract: a blob is only deletable when ref_count = 0 AND created_at is
-- older than a grace period, because the blob is inserted before the row that
-- references it. Example (run inside one transaction):
--     DELETE FROM content
--      WHERE ref_count = 0 AND created_at < unixepoch() - 3600
--     RETURNING id, storage_ref;   -- then delete external objects for storage_ref
-- -----------------------------------------------------------------------------
CREATE TABLE content (
    id              INTEGER PRIMARY KEY,
    hash            BLOB    NOT NULL,              -- blake3 of the ORIGINAL (uncompressed) bytes
    mime_type       TEXT    NOT NULL,              -- sniffed type, e.g. 'text/html'
    size_original   INTEGER NOT NULL,
    size_stored     INTEGER NOT NULL,
    compression_id  INTEGER NOT NULL DEFAULT 0 REFERENCES compression(id) ON DELETE RESTRICT,
    data            BLOB,                          -- inline bytes; NULL if stored externally
    storage_ref     TEXT,                          -- object-store key (S3/R2); NULL if inline
    ref_count       INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),

    CONSTRAINT uq_content_hash        UNIQUE (hash),
    CONSTRAINT ck_content_hash_len    CHECK (length(hash) = 32),
    CONSTRAINT ck_content_mime        CHECK (length(mime_type) > 0),
    CONSTRAINT ck_content_sizes       CHECK (size_original >= 0 AND size_stored >= 0),
    -- uncompressed content must not change size
    CONSTRAINT ck_content_none_size   CHECK (compression_id <> 0 OR size_stored = size_original),
    -- exactly one storage location
    CONSTRAINT ck_content_location    CHECK ((data IS NULL) <> (storage_ref IS NULL)),
    CONSTRAINT ck_content_inline_size CHECK (data IS NULL OR length(data) = size_stored),
    CONSTRAINT ck_content_ref_nonempty CHECK (storage_ref IS NULL OR length(storage_ref) > 0),
    -- a negative count means trigger drift: fail loudly instead of corrupting the GC
    CONSTRAINT ck_content_ref_count   CHECK (ref_count >= 0)
) STRICT;

-- Tiny partial index: exactly the rows the GC scans.
CREATE INDEX idx_content_unreferenced ON content(created_at) WHERE ref_count = 0;

-- -----------------------------------------------------------------------------
-- collection: user-organised folders. May be nested.
-- NOTE: cycle prevention beyond "not your own parent" cannot be done in a
-- trigger (SQLite forbids WITH inside CREATE TRIGGER); enforce it in the
-- application when changing parent_id.
-- -----------------------------------------------------------------------------
CREATE TABLE collection (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL,
    emoji       TEXT    NOT NULL DEFAULT '📁',
    description TEXT    NOT NULL DEFAULT '',
    color       TEXT    NOT NULL DEFAULT '#6d5dfc',
    parent_id   INTEGER REFERENCES collection(id) ON DELETE SET NULL,
    public      INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),

    CONSTRAINT ck_collection_name_nonblank CHECK (length(trim(name)) > 0),
    CONSTRAINT ck_collection_public        CHECK (public IN (0, 1)),
    CONSTRAINT ck_collection_not_own_parent CHECK (parent_id IS NULL OR parent_id <> id)
) STRICT;

-- Names are unique among siblings, not globally (ids are >= 0, so -1 is a safe
-- stand-in for "no parent", since NULLs are distinct in unique indexes).
CREATE UNIQUE INDEX uq_collection_parent_name
    ON collection(COALESCE(parent_id, -1), name COLLATE NOCASE);
CREATE INDEX idx_collection_parent ON collection(parent_id) WHERE parent_id IS NOT NULL;
CREATE INDEX idx_collection_public ON collection(id)        WHERE public = 1;

-- Built-in collection. id 0 is reserved and protected against deletion below.
INSERT INTO collection (id, name, emoji, description, color, public)
VALUES (0, 'Archive', '🗄️', 'Long-term storage for retired or inactive items.', '#6d5dfc', 0);

CREATE TRIGGER trg_collection_protect_builtin
BEFORE DELETE ON collection
WHEN OLD.id = 0
BEGIN
    SELECT RAISE(ABORT, 'the built-in Archive collection cannot be deleted');
END;

-- -----------------------------------------------------------------------------
-- crawl: optional grouping of captures by crawl job
-- -----------------------------------------------------------------------------
CREATE TABLE crawl (
    id          INTEGER PRIMARY KEY,
    name        TEXT,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    CONSTRAINT ck_crawl_times CHECK (finished_at IS NULL OR finished_at >= started_at)
) STRICT;

-- -----------------------------------------------------------------------------
-- page: one row per canonical URL
-- -----------------------------------------------------------------------------
CREATE TABLE page (
    id                   INTEGER PRIMARY KEY,
    url                  TEXT    NOT NULL,         -- canonicalized
    scheme               TEXT    NOT NULL,
    host                 TEXT    NOT NULL,         -- canonicalized => lower-case
    path                 TEXT    NOT NULL,
    query                TEXT,

    display_title        TEXT    NOT NULL DEFAULT '',   -- user-editable link title
    description          TEXT    NOT NULL DEFAULT '',   -- user-editable notes
    collection_id        INTEGER REFERENCES collection(id) ON DELETE SET NULL,
    pinned               INTEGER NOT NULL DEFAULT 0,
    thumbnail_content_id INTEGER REFERENCES content(id) ON DELETE RESTRICT,
    created_at           INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at           INTEGER NOT NULL DEFAULT (unixepoch()),

    CONSTRAINT uq_page_url      UNIQUE (url),
    CONSTRAINT ck_page_scheme   CHECK (scheme IN ('http', 'https')),
    CONSTRAINT ck_page_host     CHECK (length(host) > 0 AND host = lower(host)),
    CONSTRAINT ck_page_pinned   CHECK (pinned IN (0, 1))
) STRICT;

CREATE INDEX idx_page_host       ON page(host);
CREATE INDEX idx_page_collection ON page(collection_id, created_at DESC) WHERE collection_id IS NOT NULL;
CREATE INDEX idx_page_pinned     ON page(created_at DESC)                WHERE pinned = 1;
CREATE INDEX idx_page_thumbnail  ON page(thumbnail_content_id)           WHERE thumbnail_content_id IS NOT NULL;

CREATE TRIGGER trg_page_touch_updated_at
AFTER UPDATE ON page
FOR EACH ROW
WHEN NEW.updated_at IS OLD.updated_at
BEGIN
    UPDATE page SET updated_at = unixepoch() WHERE id = NEW.id;
END;

-- -----------------------------------------------------------------------------
-- highlight: a highlighted passage inside a saved page
-- -----------------------------------------------------------------------------
CREATE TABLE highlight (
    id          INTEGER PRIMARY KEY,
    page_id     INTEGER NOT NULL REFERENCES page(id) ON DELETE CASCADE,
    text        TEXT    NOT NULL,
    color       TEXT    NOT NULL DEFAULT 'yellow',
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    CONSTRAINT ck_highlight_text CHECK (length(text) > 0)
) STRICT;

CREATE INDEX idx_highlight_page ON highlight(page_id, created_at);

-- -----------------------------------------------------------------------------
-- capture: each time a page was fetched
-- -----------------------------------------------------------------------------
CREATE TABLE capture (
    id                    INTEGER PRIMARY KEY,
    page_id               INTEGER NOT NULL REFERENCES page(id)    ON DELETE CASCADE,
    content_id            INTEGER REFERENCES content(id)          ON DELETE RESTRICT, -- rewritten HTML served to clients (NULL for redirects / empty / errors)
    original_content_id   INTEGER REFERENCES content(id)          ON DELETE RESTRICT, -- exact bytes uploaded
    captured_at           INTEGER NOT NULL,
    title                 TEXT    NOT NULL DEFAULT '',  -- can change between captures, so stored per capture
    status_code           INTEGER,
    redirect_page_id      INTEGER REFERENCES page(id)             ON DELETE SET NULL,
    content_type          TEXT    NOT NULL DEFAULT '',  -- header as served (may differ from sniffed)
    screenshot_content_id INTEGER REFERENCES content(id)          ON DELETE RESTRICT,
    metadata              TEXT,                         -- JSON from the extension
    original_encoding     TEXT,                         -- Content-Encoding the server sent
    request_headers       TEXT,                         -- JSON
    response_headers      TEXT,                         -- JSON
    crawl_id              INTEGER REFERENCES crawl(id)            ON DELETE SET NULL,
    error                 TEXT,

    CONSTRAINT ck_capture_status   CHECK (status_code IS NULL OR status_code BETWEEN 100 AND 599),
    CONSTRAINT ck_capture_meta     CHECK (metadata         IS NULL OR json_valid(metadata)),
    CONSTRAINT ck_capture_req_hdr  CHECK (request_headers  IS NULL OR json_valid(request_headers)),
    CONSTRAINT ck_capture_resp_hdr CHECK (response_headers IS NULL OR json_valid(response_headers))
) STRICT;

CREATE INDEX idx_capture_page_time        ON capture(page_id, captured_at DESC);
CREATE INDEX idx_capture_content          ON capture(content_id)            WHERE content_id IS NOT NULL;
CREATE INDEX idx_capture_original_content ON capture(original_content_id)   WHERE original_content_id IS NOT NULL;
CREATE INDEX idx_capture_screenshot       ON capture(screenshot_content_id) WHERE screenshot_content_id IS NOT NULL;
CREATE INDEX idx_capture_redirect_page    ON capture(redirect_page_id)      WHERE redirect_page_id IS NOT NULL;
CREATE INDEX idx_capture_crawl            ON capture(crawl_id)              WHERE crawl_id IS NOT NULL;

-- -----------------------------------------------------------------------------
-- capture_asset: sub-resources (stylesheets, scripts, images, fonts, ...)
-- referenced by an archived document. One row per (capture, URL) so the
-- original URL is preserved; content_id points at the deduplicated blob.
-- -----------------------------------------------------------------------------
CREATE TABLE capture_asset (
    capture_id  INTEGER NOT NULL REFERENCES capture(id) ON DELETE CASCADE,
    content_id  INTEGER NOT NULL REFERENCES content(id) ON DELETE RESTRICT,
    url         TEXT    NOT NULL,                  -- absolute URL as referenced by the document
    kind        TEXT    NOT NULL,
    status_code INTEGER,                           -- HTTP status of the fetch, if any
    PRIMARY KEY (capture_id, url),
    CONSTRAINT ck_capture_asset_kind CHECK (kind IN
        ('stylesheet','script','image','font','media','frame','manifest','icon','other')),
    CONSTRAINT ck_capture_asset_status CHECK (status_code IS NULL OR status_code BETWEEN 100 AND 599)
) STRICT;

CREATE INDEX idx_capture_asset_content ON capture_asset(content_id);

-- -----------------------------------------------------------------------------
-- page_tag: tags attached to a page (shared by all captures)
-- -----------------------------------------------------------------------------
CREATE TABLE page_tag (
    page_id INTEGER NOT NULL REFERENCES page(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tag(id)  ON DELETE CASCADE,
    PRIMARY KEY (page_id, tag_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_page_tag_tag ON page_tag(tag_id);   -- "all pages with tag X"

-- -----------------------------------------------------------------------------
-- pending: queue of URLs waiting to be archived
--
-- Worker contract:
--   claim:   UPDATE pending SET status='in_progress', locked_at=unixepoch(),
--                   attempts=attempts+1
--             WHERE id = (SELECT id FROM pending
--                          WHERE status='queued' AND next_attempt_at <= unixepoch()
--                          ORDER BY next_attempt_at, id LIMIT 1)
--            RETURNING *;
--   recover: rows stuck 'in_progress' with an old locked_at are crashed workers;
--            reset them to 'queued'.
-- -----------------------------------------------------------------------------
CREATE TABLE pending (
    id              INTEGER PRIMARY KEY,
    url             TEXT    NOT NULL,              -- canonicalized text; not a page_id (not archived yet)
    status          TEXT    NOT NULL DEFAULT 'queued',
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    next_attempt_at INTEGER NOT NULL DEFAULT (unixepoch()),   -- retry/backoff scheduling
    locked_at       INTEGER,                       -- lease start while 'in_progress'
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),

    CONSTRAINT uq_pending_url      UNIQUE (url),
    CONSTRAINT ck_pending_status   CHECK (status IN ('queued', 'in_progress', 'failed')),
    CONSTRAINT ck_pending_attempts CHECK (attempts >= 0)
) STRICT;

-- Serves the worker poll query exactly.
CREATE INDEX idx_pending_ready  ON pending(next_attempt_at, id) WHERE status = 'queued';
CREATE INDEX idx_pending_status ON pending(status, created_at);

-- -----------------------------------------------------------------------------
-- pending_tag: tags chosen for a pending item (copied to page_tag on archive)
-- -----------------------------------------------------------------------------
CREATE TABLE pending_tag (
    pending_id  INTEGER NOT NULL REFERENCES pending(id) ON DELETE CASCADE,
    tag_id      INTEGER NOT NULL REFERENCES tag(id)     ON DELETE CASCADE,
    PRIMARY KEY (pending_id, tag_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_pending_tag_tag ON pending_tag(tag_id);

-- =============================================================================
-- content.ref_count maintenance
--
-- Kept in SQL rather than in every application call site, so deleting a
-- page/capture through any path (including FK cascades) keeps counts correct.
-- Each reference column gets its own statement on purpose: using
-- "id IN (a, b, c)" would collapse duplicates and under-count.
-- A NULL reference matches no row (id = NULL is never true), so no WHEN guard
-- is needed on inserts/deletes. Updates only touch counts when the value
-- actually changed.
-- =============================================================================

-- capture --------------------------------------------------------------------
CREATE TRIGGER trg_capture_ref_insert
AFTER INSERT ON capture
BEGIN
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.original_content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.screenshot_content_id;
END;

CREATE TRIGGER trg_capture_ref_update
AFTER UPDATE OF content_id, original_content_id, screenshot_content_id ON capture
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.content_id            AND OLD.content_id            IS NOT NEW.content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.content_id            AND NEW.content_id            IS NOT OLD.content_id;
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.original_content_id   AND OLD.original_content_id   IS NOT NEW.original_content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.original_content_id   AND NEW.original_content_id   IS NOT OLD.original_content_id;
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.screenshot_content_id AND OLD.screenshot_content_id IS NOT NEW.screenshot_content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.screenshot_content_id AND NEW.screenshot_content_id IS NOT OLD.screenshot_content_id;
END;

CREATE TRIGGER trg_capture_ref_delete
AFTER DELETE ON capture
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.original_content_id;
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.screenshot_content_id;
END;

-- capture_asset --------------------------------------------------------------
CREATE TRIGGER trg_capture_asset_ref_insert
AFTER INSERT ON capture_asset
BEGIN
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
END;

CREATE TRIGGER trg_capture_asset_ref_update
AFTER UPDATE OF content_id ON capture_asset
WHEN OLD.content_id IS NOT NEW.content_id
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
END;

CREATE TRIGGER trg_capture_asset_ref_delete
AFTER DELETE ON capture_asset
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
END;

-- page thumbnail -------------------------------------------------------------
CREATE TRIGGER trg_page_thumbnail_ref_insert
AFTER INSERT ON page
WHEN NEW.thumbnail_content_id IS NOT NULL
BEGIN
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.thumbnail_content_id;
END;

CREATE TRIGGER trg_page_thumbnail_ref_update
AFTER UPDATE OF thumbnail_content_id ON page
WHEN OLD.thumbnail_content_id IS NOT NEW.thumbnail_content_id
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.thumbnail_content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.thumbnail_content_id;
END;

CREATE TRIGGER trg_page_thumbnail_ref_delete
AFTER DELETE ON page
WHEN OLD.thumbnail_content_id IS NOT NULL
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.thumbnail_content_id;
END;

-- =============================================================================
-- Integrity audit: returns one row per blob whose stored ref_count disagrees
-- with the real number of references. Should always be empty. Run it from a
-- periodic health check or a test suite; fix drift with:
--     UPDATE content SET ref_count = <actual> WHERE id = <id>;
-- =============================================================================
CREATE VIEW v_content_ref_drift AS
SELECT c.id,
       c.ref_count            AS stored,
       COALESCE(r.n, 0)       AS actual
  FROM content c
  LEFT JOIN (
        SELECT id, SUM(n) AS n
          FROM (
                SELECT content_id            AS id, COUNT(*) AS n FROM capture       WHERE content_id            IS NOT NULL GROUP BY content_id
                UNION ALL
                SELECT original_content_id   AS id, COUNT(*) AS n FROM capture       WHERE original_content_id   IS NOT NULL GROUP BY original_content_id
                UNION ALL
                SELECT screenshot_content_id AS id, COUNT(*) AS n FROM capture       WHERE screenshot_content_id IS NOT NULL GROUP BY screenshot_content_id
                UNION ALL
                SELECT content_id            AS id, COUNT(*) AS n FROM capture_asset                                         GROUP BY content_id
                UNION ALL
                SELECT thumbnail_content_id  AS id, COUNT(*) AS n FROM page         WHERE thumbnail_content_id  IS NOT NULL GROUP BY thumbnail_content_id
          )
         GROUP BY id
       ) r ON r.id = c.id
 WHERE c.ref_count <> COALESCE(r.n, 0);

PRAGMA user_version = 1;

-- COMMIT;
