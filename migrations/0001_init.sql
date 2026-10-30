PRAGMA foreign_keys = ON;

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
    mime_type       TEXT,                      -- sniffed type, e.g. 'text/html'
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

-- Normalized URLs
CREATE TABLE url (
    id          INTEGER PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,          -- canonicalized
    scheme      TEXT NOT NULL,
    host        TEXT NOT NULL,
    path        TEXT NOT NULL,
    query       TEXT
);
CREATE INDEX idx_url_host ON url(host);

-- Each time a URL was fetched
CREATE TABLE capture (
    id              INTEGER PRIMARY KEY,
    url_id          INTEGER NOT NULL REFERENCES url(id),
    content_id      INTEGER REFERENCES content(id),   -- rewritten HTML served to clients (NULL for redirects / empty / errors)
    original_content_id INTEGER REFERENCES content(id), -- exact bytes uploaded to POST /api/archive
    captured_at     INTEGER NOT NULL,                 -- unix epoch
    title           TEXT NOT NULL,                    -- Title can change between versions, so store it per capture  
    status_code     INTEGER,
    redirect_url_id INTEGER REFERENCES url(id),
    content_type    TEXT,                             -- header as served (may differ from sniffed)
    screenshot_content_id INTEGER REFERENCES content(id),
    metadata TEXT,   -- JSON from the extension
    original_encoding TEXT,                           -- Content-Encoding the server sent
    request_headers  TEXT,                            -- JSON
    response_headers TEXT,                            -- JSON
    crawl_id        INTEGER REFERENCES crawl(id),
    error           TEXT
);
CREATE INDEX idx_capture_url_time ON capture(url_id, captured_at DESC);
CREATE INDEX idx_capture_content  ON capture(content_id);
CREATE INDEX idx_capture_orignal_content  ON capture(content_orignal_id);
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

-- Tags: global, shared across all pages and versions
CREATE TABLE tag (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,   -- 'Recipes' and 'recipes' are the same tag
    color       TEXT,                                  -- optional palette key, e.g. 'red', 'teal'; NULL = frontend decides
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Tags attached to a page (all versions share these)
CREATE TABLE url_tag (
    url_id  INTEGER NOT NULL REFERENCES url(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    PRIMARY KEY (url_id, tag_id)
);
CREATE INDEX idx_url_tag_tag ON url_tag(tag_id);   -- "all pages with tag X"

-- Queue of pages waiting to be archived
CREATE TABLE pending (
    id          INTEGER PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,      -- raw/canonicalized text, not url_id (not archived yet)
    title       TEXT,                      -- optional, user-provided hint
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
