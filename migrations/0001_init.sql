PRAGMA foreign_keys = ON;

-- Tags: global, shared across all pages and versions
CREATE TABLE tag (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,   -- 'Recipes' and 'recipes' are the same tag
    color       TEXT,                                  -- optional palette key, e.g. 'red', 'teal'; NULL = frontend decides
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Queue of pages waiting to be archived
CREATE TABLE pending (
    id          INTEGER PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,      -- raw/canonicalized text, not page_id (not archived yet)
    status      TEXT NOT NULL DEFAULT 'queued'
                CHECK (status IN ('queued','in_progress','failed')),
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    created_at    INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_pending_status ON pending(status, created_at);

-- Tags chosen for a pending item
CREATE TABLE pending_tag (
    pending_id  INTEGER NOT NULL REFERENCES pending(id) ON DELETE CASCADE,
    tag_id      INTEGER NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    PRIMARY KEY (pending_id, tag_id)
);

-- Compression algorithms: adding a new one is just an INSERT
CREATE TABLE compression (
    id    INTEGER PRIMARY KEY,
    name  TEXT NOT NULL UNIQUE        -- 'none', 'gzip', 'brotli', 'zstd', ...
);
INSERT INTO compression (id, name) VALUES (0,'none'), (1,'gzip'), (2,'brotli');

-- Deduplicated content. One row per unique file.
CREATE TABLE content (
    id              INTEGER PRIMARY KEY,
    hash            BLOB NOT NULL UNIQUE CHECK (length(hash) = 32),  -- blake3 of ORIGINAL (uncompressed) bytes
    mime_type       TEXT NOT NULL,                      -- sniffed type, e.g. 'text/html'
    size_original   INTEGER NOT NULL,
    size_stored     INTEGER NOT NULL,
    compression_id  INTEGER NOT NULL DEFAULT 0 REFERENCES compression(id),
    data            BLOB,                      -- NULL if stored externally
    storage_ref     TEXT,                      -- e.g. S3/R2 key for large files
    -- Number of capture rows and capture_asset rows pointing at this blob.
    -- Maintained by the triggers at the bottom of this file. Used by the GC:
    -- ref_count = 0 means the blob is unreferenced and may be deleted.
    ref_count       INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
    CHECK (data IS NOT NULL OR storage_ref IS NOT NULL)
);
CREATE INDEX idx_content_ref_count ON content(ref_count);

-- Collections: user-organised folders of links. Collections may be nested.
CREATE TABLE collection (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    emoji       TEXT NOT NULL DEFAULT '📁',
    description TEXT NOT NULL DEFAULT '',
    color       TEXT NOT NULL DEFAULT '#6d5dfc',
    parent_id   INTEGER REFERENCES collection(id) ON DELETE SET NULL,
    public      INTEGER NOT NULL DEFAULT 0 CHECK (public IN (0, 1)),
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_collection_parent ON collection(parent_id);
CREATE INDEX idx_collection_public ON collection(public);

INSERT INTO collection (id, name, emoji, description, color, public)
VALUES (0, 'Archive', '🗄️ ', 'Long-term storage for retired or inactive items.', '#6d5dfc', 0);

CREATE TABLE page (
    id              INTEGER PRIMARY KEY,
    url             TEXT NOT NULL UNIQUE,          -- canonicalized
    scheme          TEXT NOT NULL,
    host            TEXT NOT NULL,
    path            TEXT NOT NULL,
    query           TEXT,

    title           TEXT NOT NULL DEFAULT '',      -- user-editable link title
    description     TEXT NOT NULL DEFAULT '',      -- user-editable notes
    collection_id   INTEGER REFERENCES collection(id) ON DELETE SET NULL,
    pinned          INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    thumbnail_content_id INTEGER REFERENCES content(id),
    created_at      INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_page_host ON page(host);
CREATE INDEX idx_page_collection ON page(collection_id);
CREATE INDEX idx_page_pinned ON page(pinned, created_at DESC);

-- A highlighted passage inside a saved link.
CREATE TABLE highlight (
    id          INTEGER PRIMARY KEY,
    page_id     INTEGER NOT NULL REFERENCES page(id) ON DELETE CASCADE,
    text        TEXT NOT NULL,
    color       TEXT NOT NULL DEFAULT 'yellow',
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_highlight_page ON highlight(page_id, created_at);

-- Each time a page was fetched
CREATE TABLE capture (
    id              INTEGER PRIMARY KEY,
    page_id         INTEGER NOT NULL REFERENCES page(id),
    content_id      INTEGER REFERENCES content(id),   -- rewritten HTML served to clients (NULL for redirects / empty / errors)
    original_content_id INTEGER REFERENCES content(id), -- exact bytes uploaded to POST /api/archive
    captured_at     INTEGER NOT NULL,                 -- unix epoch
    title           TEXT NOT NULL,                    -- Title can change between versions, so store it per capture  
    status_code     INTEGER,
    redirect_page_id INTEGER REFERENCES page(id),
    content_type    TEXT NOT NULL,                    -- header as served (may differ from sniffed)
    screenshot_content_id INTEGER REFERENCES content(id),
    metadata TEXT,   -- JSON from the extension
    original_encoding TEXT,                           -- Content-Encoding the server sent
    request_headers  TEXT,                            -- JSON
    response_headers TEXT,                            -- JSON
    crawl_id        INTEGER REFERENCES crawl(id),
    error           TEXT
);
CREATE INDEX idx_capture_page_time ON capture(page_id, captured_at DESC);
CREATE INDEX idx_capture_content  ON capture(content_id);
CREATE INDEX idx_capture_orignal_content  ON capture(original_content_id);
CREATE INDEX idx_capture_crawl    ON capture(crawl_id);

-- Optional: group captures by crawl job
CREATE TABLE crawl (
    id          INTEGER PRIMARY KEY,
    name        TEXT,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER
);

-- Sub-resources (stylesheets, scripts, images, fonts, media, frames, ...)
-- referenced by an archived document. One row per (capture, URL) so the
-- original URL is preserved; content_id points at the deduplicated blob.
CREATE TABLE capture_asset (
    capture_id  INTEGER NOT NULL REFERENCES capture(id) ON DELETE CASCADE,
    content_id  INTEGER NOT NULL REFERENCES content(id),
    url         TEXT NOT NULL,                 -- absolute URL as referenced by the document
    kind        TEXT NOT NULL,                 -- stylesheet | script | image | font | media | frame | manifest | icon | other
    status_code INTEGER,                       -- HTTP status of the fetch, if any
    PRIMARY KEY (capture_id, url)
);
CREATE INDEX idx_capture_asset_content ON capture_asset(content_id);

-- Tags attached to a page (all versions share these)
CREATE TABLE page_tag (
    page_id INTEGER NOT NULL REFERENCES page(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    PRIMARY KEY (page_id, tag_id)
);
CREATE INDEX idx_page_tag_tag ON page_tag(tag_id);   -- "all pages with tag X"

-- ── content.ref_count maintenance ───────────────────────────────────────────
-- Kept in SQL rather than in every Rust call site, so deleting a page/capture
-- through any path keeps the counts correct.

CREATE TRIGGER capture_content_ref_inc_insert
AFTER INSERT ON capture
WHEN NEW.content_id IS NOT NULL
BEGIN
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
END;

CREATE TRIGGER capture_original_content_ref_inc_insert
AFTER INSERT ON capture
WHEN NEW.original_content_id IS NOT NULL
BEGIN
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.original_content_id;
END;

CREATE TRIGGER capture_screenshot_ref_inc_insert
AFTER INSERT ON capture
WHEN NEW.screenshot_content_id IS NOT NULL
BEGIN
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.screenshot_content_id;
END;

CREATE TRIGGER capture_content_ref_update
AFTER UPDATE OF content_id, original_content_id, screenshot_content_id ON capture
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.original_content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.original_content_id;
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.screenshot_content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.screenshot_content_id;
END;

CREATE TRIGGER capture_content_ref_dec_delete
AFTER DELETE ON capture
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.original_content_id;
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.screenshot_content_id;
END;

CREATE TRIGGER capture_asset_ref_inc_insert
AFTER INSERT ON capture_asset
BEGIN
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
END;

CREATE TRIGGER capture_asset_ref_update
AFTER UPDATE OF content_id ON capture_asset
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.content_id;
END;

CREATE TRIGGER capture_asset_ref_dec_delete
AFTER DELETE ON capture_asset
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.content_id;
END;

CREATE TRIGGER page_thumbnail_ref_inc_insert
AFTER INSERT ON page
WHEN NEW.thumbnail_content_id IS NOT NULL
BEGIN
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.thumbnail_content_id;
END;

CREATE TRIGGER page_thumbnail_ref_update
AFTER UPDATE OF thumbnail_content_id ON page
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.thumbnail_content_id;
    UPDATE content SET ref_count = ref_count + 1 WHERE id = NEW.thumbnail_content_id;
END;

CREATE TRIGGER page_thumbnail_ref_dec_delete
AFTER DELETE ON page
BEGIN
    UPDATE content SET ref_count = ref_count - 1 WHERE id = OLD.thumbnail_content_id;
END;
