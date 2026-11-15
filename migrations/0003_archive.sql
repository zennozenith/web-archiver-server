-- =============================================================================
-- Web archive schema, integrated with the user-management schema.
-- (SQLite >= 3.42, STRICT tables). Migration 0002: requires 0001 (users, roles,
-- permissions, role_permissions).
--
--   * PRAGMAs removed  -> set per-connection in src/db.rs (SqliteConnectOptions)
--   * No BEGIN/COMMIT  -> sqlx wraps each migration in a transaction
--   * No PRAGMA user_version -> sqlx tracks versions in _sqlx_migrations
--
-- Conventions
--   * Table names are plural (tags, pages, captures, ...); FK columns stay
--     singular (page_id, tag_id, ...).
--   * ALL timestamps are unix epoch MILLISECONDS (UTC), INTEGER, matching the
--     user schema:  CAST(round(unixepoch('subsec') * 1000) AS INTEGER)
--   * Tables are STRICT; every FK declares ON DELETE explicitly and every FK
--     column that a parent DELETE can hit is indexed.
--   * Booleans are INTEGER 0/1 with a CHECK; JSON columns use json_valid().
--
-- Ownership model
--   * tags, collections, crawls, pages and pending_items belong to one user
--     (owner_user_id, ON DELETE CASCADE). captures, highlights, capture_assets,
--     page_tags and pending_item_tags inherit ownership through their parent.
--   * Uniqueness (page url, tag name, collection name, pending url)
--     is per owner, not global.
--   * contents (blobs) stays GLOBAL and deduplicated across users. Authorise
--     blob reads in the application by checking the caller owns (or can see
--     via a public collection) a capture/page that references the blob.
--   * Cross-owner references (page in someone else's collection, tag from
--     another user, ...) are rejected by triggers, and owner_user_id is
--     immutable once set.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- tags: per user
-- -----------------------------------------------------------------------------
CREATE TABLE tags (
    id             INTEGER PRIMARY KEY,
    owner_user_id  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name           TEXT    NOT NULL COLLATE NOCASE,   -- 'Recipes' and 'recipes' are the same tag
    color          TEXT,                              -- palette key; NULL = frontend decides
    created_at     INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    CONSTRAINT uq_tag_owner_name    UNIQUE (owner_user_id, name),
    CONSTRAINT ck_tag_name_nonblank CHECK (length(name) > 0 AND name = trim(name))
) STRICT;

-- -----------------------------------------------------------------------------
-- compressions: lookup table (global). Ids are an on-disk contract: never renumber.
-- -----------------------------------------------------------------------------
CREATE TABLE compressions (
    id    INTEGER PRIMARY KEY,
    name  TEXT NOT NULL UNIQUE                     -- 'none', 'gzip', 'brotli', 'zstd', ...
) STRICT;

INSERT INTO compressions (id, name) VALUES (0, 'none'), (1, 'gzip'), (2, 'brotli');

-- -----------------------------------------------------------------------------
-- contents: deduplicated blobs, one row per unique file (global).
--
-- ref_count = number of references from captures.content_id,
-- captures.original_content_id, captures.screenshot_content_id,
-- capture_assets.content_id and pages.thumbnail_content_id (a blob referenced
-- twice by one row counts twice). Maintained exclusively by the triggers
-- below; the application must never write it. See v_content_ref_drift.
--
-- GC contract: deletable only when ref_count = 0 AND older than a grace period
-- (the blob is inserted before the row that references it). Run in one
-- transaction; the 3600000 below is 1 hour in milliseconds:
--     DELETE FROM contents
--      WHERE ref_count = 0
--        AND created_at < CAST(round(unixepoch('subsec') * 1000) AS INTEGER) - 3600000
--     RETURNING id, storage_ref;   -- then delete external objects for storage_ref
-- -----------------------------------------------------------------------------
CREATE TABLE contents (
    id              INTEGER PRIMARY KEY,
    hash            BLOB    NOT NULL,              -- blake3 of the ORIGINAL (uncompressed) bytes
    mime_type       TEXT    NOT NULL,              -- sniffed type, e.g. 'text/html'
    size_original   INTEGER NOT NULL,
    size_stored     INTEGER NOT NULL,
    compression_id  INTEGER NOT NULL DEFAULT 0 REFERENCES compressions(id) ON DELETE RESTRICT,
    data            BLOB,                          -- inline bytes; NULL if stored externally
    storage_ref     TEXT,                          -- object-store key (S3/R2); NULL if inline
    ref_count       INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),

    CONSTRAINT uq_content_hash         UNIQUE (hash),
    CONSTRAINT ck_content_hash_len     CHECK (length(hash) = 32),
    CONSTRAINT ck_content_mime         CHECK (length(mime_type) > 0),
    CONSTRAINT ck_content_sizes        CHECK (size_original >= 0 AND size_stored >= 0),
    CONSTRAINT ck_content_none_size    CHECK (compression_id <> 0 OR size_stored = size_original),
    CONSTRAINT ck_content_location     CHECK ((data IS NULL) <> (storage_ref IS NULL)),
    CONSTRAINT ck_content_inline_size  CHECK (data IS NULL OR length(data) = size_stored),
    CONSTRAINT ck_content_ref_nonempty CHECK (storage_ref IS NULL OR length(storage_ref) > 0),
    CONSTRAINT ck_content_ref_count    CHECK (ref_count >= 0)
) STRICT;

-- Tiny partial index: exactly the rows the GC scans.
CREATE INDEX ix_content_unreferenced ON contents(created_at) WHERE ref_count = 0;

-- -----------------------------------------------------------------------------
-- collections: per-user folders. Flat: collections cannot be nested.
--
-- is_builtin = 1 marks the per-user 'Archive' collection, created automatically
-- for every user (trigger below) and protected from deletion.
-- -----------------------------------------------------------------------------
CREATE TABLE collections (
    id             INTEGER PRIMARY KEY,
    owner_user_id  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    emoji          TEXT    NOT NULL DEFAULT '📁',
    description    TEXT    NOT NULL DEFAULT '',
    color          TEXT    NOT NULL DEFAULT '#6d5dfc',
    public         INTEGER NOT NULL DEFAULT 0,
    is_builtin     INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),

    CONSTRAINT ck_collection_name_nonblank CHECK (length(trim(name)) > 0),
    CONSTRAINT ck_collection_public        CHECK (public IN (0, 1)),
    CONSTRAINT ck_collection_builtin       CHECK (is_builtin IN (0, 1))
) STRICT;

-- Names are unique per owner, case-insensitively.
CREATE UNIQUE INDEX ux_collection_owner_name
    ON collections(owner_user_id, name COLLATE NOCASE);
-- Exactly one built-in collection per user.
CREATE UNIQUE INDEX ux_collection_builtin_per_owner
    ON collections(owner_user_id) WHERE is_builtin = 1;
CREATE INDEX ix_collection_public ON collections(id) WHERE public = 1;

-- Cannot delete the built-in collection, unless its owner is already gone
-- (SQLite runs FK cascade actions after the parent row is deleted, so a
-- cascade from a hard-deleted user is allowed through).
CREATE TRIGGER trg_collection_protect_builtin
BEFORE DELETE ON collections
FOR EACH ROW WHEN OLD.is_builtin = 1
                AND EXISTS (SELECT 1 FROM users WHERE id = OLD.owner_user_id)
BEGIN
    SELECT RAISE(ABORT, 'the built-in Archive collection cannot be deleted');
END;

-- Auto-create the built-in collection for every new user.
CREATE TRIGGER trg_users_create_archive_collection
AFTER INSERT ON users
BEGIN
    INSERT INTO collections (owner_user_id, name, emoji, description, color, public, is_builtin)
    VALUES (NEW.id, 'Archive', '🗄️', 'Long-term storage for retired or inactive items.', '#6d5dfc', 0, 1);
END;

-- Backfill for users that already exist when this migration runs.
INSERT INTO collections (owner_user_id, name, emoji, description, color, public, is_builtin)
SELECT id, 'Archive', '🗄️', 'Long-term storage for retired or inactive items.', '#6d5dfc', 0, 1
  FROM users;

-- -----------------------------------------------------------------------------
-- crawls: optional grouping of captures by crawl job (per user)
-- -----------------------------------------------------------------------------
CREATE TABLE crawls (
    id             INTEGER PRIMARY KEY,
    owner_user_id  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name           TEXT,
    started_at     INTEGER NOT NULL,
    finished_at    INTEGER,
    CONSTRAINT ck_crawl_times CHECK (finished_at IS NULL OR finished_at >= started_at)
) STRICT;
CREATE INDEX ix_crawl_owner ON crawls(owner_user_id, started_at DESC);

-- -----------------------------------------------------------------------------
-- pages: one row per canonical URL per user
-- -----------------------------------------------------------------------------
CREATE TABLE pages (
    id                   INTEGER PRIMARY KEY,
    owner_user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    url                  TEXT    NOT NULL,         -- canonicalized
    scheme               TEXT    NOT NULL,
    host                 TEXT    NOT NULL,         -- canonicalized => lower-case
    path                 TEXT    NOT NULL,
    query                TEXT,

    display_title        TEXT    NOT NULL DEFAULT '',   -- user-editable link title
    description          TEXT    NOT NULL DEFAULT '',   -- user-editable notes
    collection_id        INTEGER REFERENCES collections(id) ON DELETE SET NULL,
    pinned               INTEGER NOT NULL DEFAULT 0,
    thumbnail_content_id INTEGER REFERENCES contents(id) ON DELETE RESTRICT,
    created_at           INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    updated_at           INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),

    CONSTRAINT uq_page_owner_url UNIQUE (owner_user_id, url),
    CONSTRAINT ck_page_scheme    CHECK (scheme IN ('http', 'https')),
    CONSTRAINT ck_page_host      CHECK (length(host) > 0 AND host = lower(host)),
    CONSTRAINT ck_page_pinned    CHECK (pinned IN (0, 1))
) STRICT;

CREATE INDEX ix_page_owner_created ON pages(owner_user_id, created_at DESC);
CREATE INDEX id_page_owner_host    ON pages(owner_user_id, host);
CREATE INDEX id_page_collection    ON pages(collection_id, created_at DESC) WHERE collection_id IS NOT NULL;
CREATE INDEX id_page_pinned        ON pages(owner_user_id, created_at DESC) WHERE pinned = 1;
CREATE INDEX id_page_thumbnail     ON pages(thumbnail_content_id)           WHERE thumbnail_content_id IS NOT NULL;

CREATE TRIGGER trg_page_touch_updated_at
AFTER UPDATE ON pages
FOR EACH ROW
WHEN NEW.updated_at IS OLD.updated_at
BEGIN
    UPDATE pages SET updated_at = CAST(round(unixepoch('subsec') * 1000) AS INTEGER) WHERE id = NEW.id;
END;

-- -----------------------------------------------------------------------------
-- highlights: a highlighted passage inside a saved page (owner = page owner)
-- -----------------------------------------------------------------------------
CREATE TABLE highlights (
    id          INTEGER PRIMARY KEY,
    page_id     INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    text        TEXT    NOT NULL,
    color       TEXT    NOT NULL DEFAULT 'yellow',
    created_at  INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    CONSTRAINT ck_highlight_text CHECK (length(text) > 0)
) STRICT;

CREATE INDEX id_highlight_page ON highlights(page_id, created_at);

-- -----------------------------------------------------------------------------
-- captures: each time a page was fetched (owner = page owner)
-- -----------------------------------------------------------------------------
CREATE TABLE captures (
    id                    INTEGER PRIMARY KEY,
    page_id               INTEGER NOT NULL REFERENCES pages(id)    ON DELETE CASCADE,
    content_id            INTEGER REFERENCES contents(id)          ON DELETE RESTRICT, -- rewritten HTML served to clients (NULL for redirects / empty / errors)
    original_content_id   INTEGER REFERENCES contents(id)          ON DELETE RESTRICT, -- exact bytes uploaded
    captured_at           INTEGER NOT NULL,
    title                 TEXT    NOT NULL DEFAULT '',  -- can change between captures, so stored per capture
    status_code           INTEGER,
    redirect_page_id      INTEGER REFERENCES pages(id)             ON DELETE SET NULL,
    content_type          TEXT    NOT NULL DEFAULT '',  -- header as served (may differ from sniffed)
    screenshot_content_id INTEGER REFERENCES contents(id)          ON DELETE RESTRICT,
    metadata              TEXT,                         -- JSON from the extension
    original_encoding     TEXT,                         -- Content-Encoding the server sent
    request_headers       TEXT,                         -- JSON
    response_headers      TEXT,                         -- JSON
    crawl_id              INTEGER REFERENCES crawls(id)            ON DELETE SET NULL,
    error                 TEXT,

    CONSTRAINT ck_capture_status   CHECK (status_code IS NULL OR status_code BETWEEN 100 AND 599),
    CONSTRAINT ck_capture_meta     CHECK (metadata         IS NULL OR json_valid(metadata)),
    CONSTRAINT ck_capture_req_hdr  CHECK (request_headers  IS NULL OR json_valid(request_headers)),
    CONSTRAINT ck_capture_resp_hdr CHECK (response_headers IS NULL OR json_valid(response_headers))
) STRICT;

CREATE INDEX id_capture_page_time        ON captures(page_id, captured_at DESC);
CREATE INDEX id_capture_content          ON captures(content_id)            WHERE content_id IS NOT NULL;
CREATE INDEX id_capture_original_content ON captures(original_content_id)   WHERE original_content_id IS NOT NULL;
CREATE INDEX id_capture_screenshot       ON captures(screenshot_content_id) WHERE screenshot_content_id IS NOT NULL;
CREATE INDEX id_capture_redirect_page    ON captures(redirect_page_id)      WHERE redirect_page_id IS NOT NULL;
CREATE INDEX id_capture_crawl            ON captures(crawl_id)              WHERE crawl_id IS NOT NULL;

-- -----------------------------------------------------------------------------
-- capture_assets: sub-resources (stylesheets, scripts, images, fonts, ...)
-- One row per (capture, URL); content_id points at the deduplicated blob.
-- -----------------------------------------------------------------------------
CREATE TABLE capture_assets (
    capture_id  INTEGER NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
    content_id  INTEGER NOT NULL REFERENCES contents(id) ON DELETE RESTRICT,
    url         TEXT    NOT NULL,                  -- absolute URL as referenced by the document
    kind        TEXT    NOT NULL,
    status_code INTEGER,                           -- HTTP status of the fetch, if any
    PRIMARY KEY (capture_id, url),
    CONSTRAINT ck_capture_asset_kind CHECK (kind IN
        ('stylesheet','script','image','font','media','frame','manifest','icon','other')),
    CONSTRAINT ck_capture_asset_status CHECK (status_code IS NULL OR status_code BETWEEN 100 AND 599)
) STRICT;

CREATE INDEX id_capture_asset_content ON capture_assets(content_id);

-- -----------------------------------------------------------------------------
-- page_tags: tags attached to a page (shared by all captures)
-- -----------------------------------------------------------------------------
CREATE TABLE page_tags (
    page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tags(id)  ON DELETE CASCADE,
    PRIMARY KEY (page_id, tag_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX id_page_tag_tag ON page_tags(tag_id);   -- "all pages with tag X"

-- -----------------------------------------------------------------------------
-- pending_items: queue of URLs waiting to be archived (per user)
--
-- Worker contract (now = CAST(round(unixepoch('subsec') * 1000) AS INTEGER)):
--   claim:   UPDATE pending_items SET status='in_progress', locked_at=<now>,
--                   attempts=attempts+1
--             WHERE id = (SELECT id FROM pending_items
--                          WHERE status='queued' AND next_attempt_at <= <now>
--                          ORDER BY next_attempt_at, id LIMIT 1)
--            RETURNING *;
--   recover: rows stuck 'in_progress' with an old locked_at are crashed workers;
--            reset them to 'queued'.
-- -----------------------------------------------------------------------------
CREATE TABLE pending_items (
    id              INTEGER PRIMARY KEY,
    owner_user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    url             TEXT    NOT NULL,              -- canonicalized text; not a page_id (not archived yet)
    status          TEXT    NOT NULL DEFAULT 'queued',
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    next_attempt_at INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    locked_at       INTEGER,                       -- lease start while 'in_progress'
    created_at      INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),

    CONSTRAINT uq_pending_owner_url UNIQUE (owner_user_id, url),
    CONSTRAINT ck_pending_status    CHECK (status IN ('queued', 'in_progress', 'failed')),
    CONSTRAINT ck_pending_attempts  CHECK (attempts >= 0)
) STRICT;

-- Serves the worker poll query exactly (global queue across users).
CREATE INDEX id_pending_ready  ON pending_items(next_attempt_at, id) WHERE status = 'queued';
CREATE INDEX id_pending_status ON pending_items(status, created_at);

-- -----------------------------------------------------------------------------
-- pending_item_tags: tags chosen for a pending item (copied to page_tags on archive)
-- -----------------------------------------------------------------------------
CREATE TABLE pending_item_tags (
    pending_item_id  INTEGER NOT NULL REFERENCES pending_items(id) ON DELETE CASCADE,
    tag_id           INTEGER NOT NULL REFERENCES tags(id)          ON DELETE CASCADE,
    PRIMARY KEY (pending_item_id, tag_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX id_pending_tag_tag ON pending_item_tags(tag_id);

-- =============================================================================
-- OWNERSHIP INTEGRITY
--
-- (a) owner_user_id is immutable. (b) A row may only reference rows of the same
-- owner. Composite FKs cannot do (b) because ON DELETE SET NULL would also try
-- to null the NOT NULL owner column, so triggers are used instead.
-- =============================================================================

-- (a) immutable owner -----------------------------------------------------------
CREATE TRIGGER trg_tag_owner_immutable BEFORE UPDATE OF owner_user_id ON tags
FOR EACH ROW WHEN NEW.owner_user_id <> OLD.owner_user_id
BEGIN SELECT RAISE(ABORT, 'tags.owner_user_id is immutable'); END;

CREATE TRIGGER trg_collection_owner_immutable BEFORE UPDATE OF owner_user_id ON collections
FOR EACH ROW WHEN NEW.owner_user_id <> OLD.owner_user_id
BEGIN SELECT RAISE(ABORT, 'collections.owner_user_id is immutable'); END;

CREATE TRIGGER trg_crawl_owner_immutable BEFORE UPDATE OF owner_user_id ON crawls
FOR EACH ROW WHEN NEW.owner_user_id <> OLD.owner_user_id
BEGIN SELECT RAISE(ABORT, 'crawls.owner_user_id is immutable'); END;

CREATE TRIGGER trg_page_owner_immutable BEFORE UPDATE OF owner_user_id ON pages
FOR EACH ROW WHEN NEW.owner_user_id <> OLD.owner_user_id
BEGIN SELECT RAISE(ABORT, 'pages.owner_user_id is immutable'); END;

CREATE TRIGGER trg_pending_owner_immutable BEFORE UPDATE OF owner_user_id ON pending_items
FOR EACH ROW WHEN NEW.owner_user_id <> OLD.owner_user_id
BEGIN SELECT RAISE(ABORT, 'pending_items.owner_user_id is immutable'); END;

-- (b) same-owner references -----------------------------------------------------
CREATE TRIGGER trg_page_collection_owner_ins BEFORE INSERT ON pages
FOR EACH ROW WHEN NEW.collection_id IS NOT NULL
 AND NOT EXISTS (SELECT 1 FROM collections WHERE id = NEW.collection_id AND owner_user_id = NEW.owner_user_id)
BEGIN SELECT RAISE(ABORT, 'collection belongs to a different user'); END;

CREATE TRIGGER trg_page_collection_owner_upd BEFORE UPDATE OF collection_id ON pages
FOR EACH ROW WHEN NEW.collection_id IS NOT NULL
 AND NOT EXISTS (SELECT 1 FROM collections WHERE id = NEW.collection_id AND owner_user_id = NEW.owner_user_id)
BEGIN SELECT RAISE(ABORT, 'collection belongs to a different user'); END;

CREATE TRIGGER trg_page_tag_owner BEFORE INSERT ON page_tags
FOR EACH ROW WHEN (SELECT owner_user_id FROM pages WHERE id = NEW.page_id)
              IS NOT (SELECT owner_user_id FROM tags  WHERE id = NEW.tag_id)
BEGIN SELECT RAISE(ABORT, 'page and tag belong to different users'); END;

CREATE TRIGGER trg_pending_tag_owner BEFORE INSERT ON pending_item_tags
FOR EACH ROW WHEN (SELECT owner_user_id FROM pending_items WHERE id = NEW.pending_item_id)
              IS NOT (SELECT owner_user_id FROM tags       WHERE id = NEW.tag_id)
BEGIN SELECT RAISE(ABORT, 'pending item and tag belong to different users'); END;

CREATE TRIGGER trg_capture_owner_ins BEFORE INSERT ON captures
FOR EACH ROW WHEN
    (NEW.redirect_page_id IS NOT NULL
       AND (SELECT owner_user_id FROM pages WHERE id = NEW.redirect_page_id)
           IS NOT (SELECT owner_user_id FROM pages WHERE id = NEW.page_id))
 OR (NEW.crawl_id IS NOT NULL
       AND (SELECT owner_user_id FROM crawls WHERE id = NEW.crawl_id)
           IS NOT (SELECT owner_user_id FROM pages WHERE id = NEW.page_id))
BEGIN SELECT RAISE(ABORT, 'capture references a page or crawl of a different user'); END;

CREATE TRIGGER trg_capture_owner_upd BEFORE UPDATE OF redirect_page_id, crawl_id ON captures
FOR EACH ROW WHEN
    (NEW.redirect_page_id IS NOT NULL
       AND (SELECT owner_user_id FROM pages WHERE id = NEW.redirect_page_id)
           IS NOT (SELECT owner_user_id FROM pages WHERE id = NEW.page_id))
 OR (NEW.crawl_id IS NOT NULL
       AND (SELECT owner_user_id FROM crawls WHERE id = NEW.crawl_id)
           IS NOT (SELECT owner_user_id FROM pages WHERE id = NEW.page_id))
BEGIN SELECT RAISE(ABORT, 'capture references a page or crawl of a different user'); END;

-- =============================================================================
-- contents.ref_count maintenance
--
-- Kept in SQL so deleting a page/capture through any path (including FK
-- cascades from a deleted user) keeps counts correct. Each reference column
-- gets its own statement on purpose: "id IN (a, b, c)" would collapse
-- duplicates and under-count. A NULL reference matches no row.
-- =============================================================================

-- captures ---------------------------------------------------------------------
CREATE TRIGGER trg_capture_ref_insert
AFTER INSERT ON captures
BEGIN
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.original_content_id;
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.screenshot_content_id;
END;

CREATE TRIGGER trg_capture_ref_update
AFTER UPDATE OF content_id, original_content_id, screenshot_content_id ON captures
BEGIN
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.content_id            AND OLD.content_id            IS NOT NEW.content_id;
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.content_id            AND NEW.content_id            IS NOT OLD.content_id;
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.original_content_id   AND OLD.original_content_id   IS NOT NEW.original_content_id;
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.original_content_id   AND NEW.original_content_id   IS NOT OLD.original_content_id;
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.screenshot_content_id AND OLD.screenshot_content_id IS NOT NEW.screenshot_content_id;
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.screenshot_content_id AND NEW.screenshot_content_id IS NOT OLD.screenshot_content_id;
END;

CREATE TRIGGER trg_capture_ref_delete
AFTER DELETE ON captures
BEGIN
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.original_content_id;
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.screenshot_content_id;
END;

-- capture_assets ---------------------------------------------------------------
CREATE TRIGGER trg_capture_asset_ref_insert
AFTER INSERT ON capture_assets
BEGIN
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
END;

CREATE TRIGGER trg_capture_asset_ref_update
AFTER UPDATE OF content_id ON capture_assets
WHEN OLD.content_id IS NOT NEW.content_id
BEGIN
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
END;

CREATE TRIGGER trg_capture_asset_ref_delete
AFTER DELETE ON capture_assets
BEGIN
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
END;

-- page thumbnail ---------------------------------------------------------------
CREATE TRIGGER trg_page_thumbnail_ref_insert
AFTER INSERT ON pages
WHEN NEW.thumbnail_content_id IS NOT NULL
BEGIN
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.thumbnail_content_id;
END;

CREATE TRIGGER trg_page_thumbnail_ref_update
AFTER UPDATE OF thumbnail_content_id ON pages
WHEN OLD.thumbnail_content_id IS NOT NEW.thumbnail_content_id
BEGIN
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.thumbnail_content_id;
    UPDATE contents SET ref_count = ref_count + 1 WHERE id = NEW.thumbnail_content_id;
END;

CREATE TRIGGER trg_page_thumbnail_ref_delete
AFTER DELETE ON pages
WHEN OLD.thumbnail_content_id IS NOT NULL
BEGIN
    UPDATE contents SET ref_count = ref_count - 1 WHERE id = OLD.thumbnail_content_id;
END;

-- =============================================================================
-- Integrity audit: one row per blob whose stored ref_count disagrees with the
-- real number of references. Should always be empty. Fix drift with:
--     UPDATE contents SET ref_count = <actual> WHERE id = <id>;
-- =============================================================================
CREATE VIEW v_content_ref_drift AS
SELECT c.id,
       c.ref_count            AS stored,
       COALESCE(r.n, 0)       AS actual
  FROM contents c
  LEFT JOIN (
        SELECT id, SUM(n) AS n
          FROM (
                SELECT content_id            AS id, COUNT(*) AS n FROM captures       WHERE content_id            IS NOT NULL GROUP BY content_id
                UNION ALL
                SELECT original_content_id   AS id, COUNT(*) AS n FROM captures       WHERE original_content_id   IS NOT NULL GROUP BY original_content_id
                UNION ALL
                SELECT screenshot_content_id AS id, COUNT(*) AS n FROM captures       WHERE screenshot_content_id IS NOT NULL GROUP BY screenshot_content_id
                UNION ALL
                SELECT content_id            AS id, COUNT(*) AS n FROM capture_assets                                         GROUP BY content_id
                UNION ALL
                SELECT thumbnail_content_id  AS id, COUNT(*) AS n FROM pages          WHERE thumbnail_content_id  IS NOT NULL GROUP BY thumbnail_content_id
          )
         GROUP BY id
       ) r ON r.id = c.id
 WHERE c.ref_count <> COALESCE(r.n, 0);
