#!/usr/bin/env fish
# seed.fish — interactive seeder for the seed-db SQLite schema (0001_init.sql).
#
# fish only orchestrates; ALL data generation lives in one SQLite script
# (recursive CTEs generate random rows — no fish loops, one transaction, FK-safe order).
# JSON columns use SQLite's json1; uniqueness across runs via random tokens.
#
# CLIs:   sqlite3 (>= 3.38)   gum (charmbracelet, all interactive UX)
# Usage:  seed.fish [--db FILE] [--schema FILE] [--mode demo|random|pick|pending]
#                   [--count N] [--wipe] [--yes]
#           demo     curated sample data (fixed ids, idempotent)
#           random   bulk generated data (--count = number of urls)
#           pick     interactively choose table groups
#           pending  only the pending queue + tag links
#         --wipe   delete existing rows before seeding (prompts unless --yes)

argparse h/help 'd/db=' 's/schema=' 'm/mode=' 'c/count=' y/yes w/wipe -- $argv
or exit 1

if set -q _flag_help
    echo "usage: seed.fish [--db FILE] [--schema FILE] [--mode demo|random|pick|pending] [--count N] [--wipe] [--yes]"
    exit 0
end

# ── deps ────────────────────────────────────────────────────────────────────
function need -a cmd
    if not command -sq $cmd
        set -l hint ""
        switch $cmd
            case sqlite3
                set hint "brew install sqlite3 | sudo apt install sqlite3 | dnf install sqlite3"
            case gum
                set hint "brew install gum | see https://github.com/charmbracelet/gum#installation"
        end
        echo "missing: $cmd" >&2
        echo "  install → $hint" >&2
        exit 1
    end
end
need sqlite3
need gum
if not sqlite3 :memory: "SELECT unixepoch();" >/dev/null 2>&1
    echo "sqlite3 is too old: unixepoch() needs v3.38+" >&2
    exit 1
end

function log -S
    gum style --foreground 39 -- " ▸ $argv"
end
function fail -S
    gum style --foreground 1 --bold -- " ✗ $argv"
    exit 1
end

# ── resolve paths, flags ────────────────────────────────────────────────────
set -l script_dir (path resolve (dirname (status current-filename)))
set -l db $_flag_db
set -l schema $_flag_schema
set -l mode $_flag_mode
set -l n_base $_flag_count
set -q db[1]; or set db seed.sqlite3
set -q schema[1]; or set schema $script_dir/0001_init.sql

# ── interactive: mode + scale ───────────────────────────────────────────────
if not set -q mode[1]
    set mode (gum choose --header "Seeding mode" --height 5 \
        "demo — curated sample data" "random — bulk generated data" \
        "pick — choose table groups" "pending — pending queue only")
    set mode (string split ' —' -- $mode)[1]
end
if not contains $mode demo random pick pending
    fail "unknown mode: $mode"
end

if not set -q n_base[1]; and test "$mode" != demo; and test "$mode" != pending
    set -l pick (gum choose --header "Scale" --height 5 \
        "S (25 urls)" "M (120 urls)" "L (480 urls)" "XL (1600 urls)")
    set -l letter (string split ' ' -- $pick)[1]
    switch $letter
        case S
            set n_base 25
        case M
            set n_base 120
        case L
            set n_base 480
        case '*'
            set n_base 1600
    end
end
set -q n_base[1]; or set n_base 25

# ── row-count targets ───────────────────────────────────────────────────────
set -g n_urls $n_base
set -g n_caps (math "$n_urls * 3")
set -g n_content (math "floor($n_caps * 4 / 5)")
set -g n_links (math "floor($n_caps * 2 / 5)")
set -g n_pen (math "floor($n_urls / 4)")
test $n_pen -lt 3; and set -g n_pen 3

# ── schema: apply 0001_init.sql only if not applied yet ─────────────────────
set -l tables_present (sqlite3 "$db" \
    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN
     ('compression','crawl','url','content','capture','tag','pending');" 2>/dev/null)
switch $tables_present
    case 7
        log "schema already applied ✓"
    case 0
        if not test -f "$schema"
            fail "schema not found: $schema (pass --schema FILE)"
        end
        if gum spin --spinner dot --title "applying $schema" -- sqlite3 -bail "$db" <"$schema"
            log "schema applied ✓"
        else
            echo "schema apply failed — see output above" >&2
            exit 1
        end
    case '*'
        fail "schema partially applied ($tables_present/7 tables) — inspect $db"
end

# ── wipe / additive seeding ─────────────────────────────────────────────────
set -l rows_before (sqlite3 "$db" \
    "SELECT (SELECT count(*) FROM crawl)
          +(SELECT count(*) FROM url)+(SELECT count(*) FROM content)
          +(SELECT count(*) FROM capture)+(SELECT count(*) FROM capture_resource)
          +(SELECT count(*) FROM tag)+(SELECT count(*) FROM url_tag)
          +(SELECT count(*) FROM pending)+(SELECT count(*) FROM pending_tag);")
if test "$rows_before" -gt 0
    if set -q _flag_wipe
        if set -q _flag_yes; or gum confirm "WIPE all rows in $db?"
            printf '%s\n' "PRAGMA foreign_keys = ON;
BEGIN;
DELETE FROM capture_resource; DELETE FROM capture;
DELETE FROM url_tag; DELETE FROM pending_tag;
DELETE FROM pending; DELETE FROM url;
DELETE FROM tag; DELETE FROM crawl;
DELETE FROM content; DELETE FROM compression;
COMMIT;" | sqlite3 -bail "$db"
            log "wiped ✓"
        else
            log "wipe declined — aborting"
            exit 0
        end
    else if not set -q _flag_yes
        gum confirm "DB has $rows_before rows — seed additively on top?" >/dev/null; or fail aborted
        log "existing data kept — new rows will be added on top (use --wipe to reset)"
    else
        log "existing data kept — new rows will be added on top"
    end
end

# ── group selection for pick mode (+ auto prerequisites) ───────────────────
set -g picks
if test "$mode" = random
    set picks crawls urls content captures capture_links tags url_tags pending pending_tags
else if test "$mode" = pending
    set picks tags pending pending_tags
else if test "$mode" = pick
    set picks (gum choose --no-limit --header "Pick groups (space to toggle, enter to run)" \
        --height 12 crawls urls content captures capture_links tags url_tags pending pending_tags)
    if test -z "$picks"
        log "nothing selected — bye"
        exit 0
    end
    # auto-add prerequisites (urls, tags, captures, pending)
    if contains captures $picks; or contains capture_links $picks
        contains urls $picks; or set -a picks urls
    end
    if contains capture_links $picks
        contains captures $picks; or set -a picks captures
    end
    if contains url_tags $picks; or contains pending_tags $picks
        contains tags $picks; or set -a picks tags
    end
    if contains pending_tags $picks
        contains pending $picks; or set -a picks pending
    end
    log "groups: $picks"
end

# ── SQL block generators (SQL does the heavy lifting; fish substitutes counts) ─
function sql_compression
    printf '%s\n' "INSERT OR IGNORE INTO compression(id, name)
    VALUES (0,'none'), (1,'gzip'), (2,'brotli'), (3,'zstd');"
end

function sql_crawl
    printf '%s\n' "WITH RECURSIVE i(n) AS (VALUES (1) UNION ALL SELECT n + 1 FROM i WHERE i.n < 3 + ($n_urls % 5))
INSERT INTO crawl(name, started_at, finished_at)
SELECT 'seed-crawl-' || t.rn || '-' || substr(t.tag, 1, 4),
       t.started,
       CASE WHEN t.rn % 3 = 0 THEN NULL ELSE t.started + 900 + ((random() & 4294967295) % 50000) END
FROM (SELECT i.n AS rn, hex(randomblob(3)) AS tag,
             unixepoch() - (i.n * 86400) - ((random() & 4294967295) % 72000) AS started
      FROM i) t;"
end

function sql_urls
    printf '%s\n' "CREATE TEMP TABLE pool_url(scheme TEXT, host TEXT, path TEXT);
INSERT INTO pool_url VALUES
  ('https', 'example.com', '/'),
  ('https', 'example.com', '/blog'),
  ('https', 'example.com', '/blog/2026'),
  ('https', 'example.com', '/docs'),
  ('https', 'example.com', '/about'),
  ('https', 'news.ycombinator.com', '/item'),
  ('https', 'news.ycombinator.com', '/newest'),
  ('https', 'sqlite.org', '/lang_UPSERT.html'),
  ('https', 'sqlite.org', '/json1.html'),
  ('https', 'en.wikipedia.org', '/wiki/Web_archiving'),
  ('https', 'en.wikipedia.org', '/wiki/Content_management_system'),
  ('https', 'github.com', '/charmbracelet/gum'),
  ('https', 'github.com', '/sqlite/sqlite'),
  ('https', 'caniuse.com', '/'),
  ('https', 'developer.mozilla.org', '/en-US/docs/Web/API'),
  ('https', 'fishshell.com', '/docs/current/'),
  ('https', 'curl.se', '/libcurl/c/');
WITH RECURSIVE i(n) AS (VALUES (1) UNION ALL SELECT n + 1 FROM i WHERE i.n < $n_urls),
gen(rn, pi, sq) AS MATERIALIZED (
    SELECT i.n,
           1 + ((random() & 4294967295) % (SELECT count(*) FROM pool_url)),
           substr(hex(randomblob(6)), 1, 10)
    FROM i
)
INSERT OR IGNORE INTO url(url, scheme, host, path, query)
SELECT CASE g.rn % 4
         WHEN 0 THEN pu.scheme || '://' || pu.host || pu.path || '/~seed-' || g.sq || '.html'
         ELSE pu.scheme || '://' || pu.host || pu.path || '?seed=' || g.sq END,
       pu.scheme, pu.host, pu.path,
       CASE g.rn % 4 WHEN 0 THEN NULL ELSE 'seed=' || g.sq END
FROM gen g JOIN pool_url pu ON pu.rowid = g.pi;"
end

function sql_content
    printf '%s\n' "WITH RECURSIVE i(n) AS (VALUES (1) UNION ALL SELECT n + 1 FROM i WHERE i.n < $n_content),
d(rn, sz, mt, comp) AS MATERIALIZED (
    SELECT i.n,
           500 + ((random() & 4294967295) % 400000),
           (random() & 4294967295) % 3,
           abs((random() & 4294967295) % 3) % 3
    FROM i
)
INSERT INTO content(hash, mime_type, size_original, size_stored, compression_id, data, storage_ref)
SELECT randomblob(32),
       CASE d.mt WHEN 0 THEN 'text/html' WHEN 1 THEN 'application/javascript' ELSE 'image/png' END,
       d.sz,
       CASE d.comp WHEN 0 THEN d.sz WHEN 1 THEN d.sz * 34 / 100 ELSE d.sz * 62 / 100 END,
       d.comp,
       CASE WHEN d.comp = 0 AND d.sz < 3000 THEN randomblob(d.sz) END,
       'blk3:' || hex(randomblob(16))
FROM d;"
end

function sql_caps
    printf '%s\n' "CREATE TEMP TABLE pool_title(t TEXT);
INSERT INTO pool_title VALUES
 ('Example Domain'), ('SQLite JSON1 functions'), ('Async Rust deep dive'),
 ('How we built Wormhole.io'), ('Changelog — v1.0.2'), ('Web archiving in 2026'),
 ('JSX: conspicuous laziness'), ('Notes on the ONE BIG FILE'), ('Bluesky: audience graph'),
 ('Local-first apps in 2026'), ('A quiet read-only API'), ('Statement storage');
WITH RECURSIVE i(n) AS (VALUES (1) UNION ALL SELECT n + 1 FROM i WHERE i.n < $n_caps),
gen(rn, url_i, cont_i, redir_i, shot_i, crawl_i, title_i, ns, oe, age, ehex) AS MATERIALIZED (
    SELECT i.n,
           ((random() & 4294967295) % max(1, (SELECT count(*) FROM url))),
           ((random() & 4294967295) % max(1, (SELECT count(*) FROM content))),
           ((random() & 4294967295) % max(1, (SELECT count(*) FROM url))),
           ((random() & 4294967295) % max(1, (SELECT count(*) FROM content))),
           ((random() & 4294967295) % max(1, (SELECT count(*) FROM crawl))),
           ((random() & 4294967295) % (SELECT count(*) FROM pool_title)),
           (random() & 4294967295) % 20,
           (random() & 4294967295) % 3,
           500 + ((random() & 4294967295) % 2600000),
           substr(hex(randomblob(8)), 1, 10)
    FROM i
)
INSERT INTO capture(page_id, content_id, captured_at, title, status_code, redirect_page_id,
                    content_type, metadata, original_encoding, request_headers, response_headers,
                    crawl_id, error, screenshot_content_id)
SELECT
  uu.upick,
  CASE WHEN g.status = 200 THEN cc.cpick END,
  unixepoch() - g.age,
  tt.tpick,
  g.status,
  CASE WHEN g.status = 301 THEN ur.rpick END,
  CASE WHEN g.status = 200 THEN 'text/html; charset=utf-8'
       WHEN g.status = 301 THEN 'text/html'
       WHEN g.status = 429 THEN 'application/json' END,
  json_object('seeded', json('true'), 'depth', (random() & 4294967295) % 6,
              'word_count', 300 + ((random() & 4294967295) % 4000), 'seq', g.rn),
  CASE WHEN g.status = 200 THEN (CASE g.oe WHEN 0 THEN 'gzip' WHEN 1 THEN 'br' ELSE 'identity' END) END,
  json_object('user-agent', 'SeedBot/1.0', 'accept', 'text/html,application/xhtml+xml,*/*;q=0.8', 'hash', g.ehex),
  json_object('server', 'seed-nginx/1.25', 'x-cache', (CASE g.rn % 3 WHEN 0 THEN 'HIT' WHEN 1 THEN 'MISS' ELSE 'BYPASS' END)),
  CASE WHEN g.status != 200 THEN cr.cpick END,
  CASE g.status
    WHEN 404 THEN 'simulated 404 ' || g.ehex
    WHEN 429 THEN 'simulated 429 ' || g.ehex
    WHEN 500 THEN 'simulated 500 ' || g.ehex END,
  CASE WHEN g.rn % 13 = 0 THEN sc.cpick END
FROM (SELECT gen.*, CASE WHEN ns < 15 THEN 200 WHEN ns = 15 THEN 301 WHEN ns = 16 THEN 404
                          WHEN ns = 17 THEN 429 ELSE 500 END AS status
      FROM gen) g
LEFT JOIN (SELECT id AS upick,   row_number() OVER () AS rn FROM url)     uu ON uu.rn = 1 + g.url_i
LEFT JOIN (SELECT id AS rpick,   row_number() OVER (ORDER BY id DESC) AS rn FROM url) ur ON ur.rn = 1 + g.redir_i
LEFT JOIN (SELECT id AS cpick,   row_number() OVER () AS rn FROM content) cc ON cc.rn = 1 + g.cont_i
LEFT JOIN (SELECT id AS cpick,   row_number() OVER () AS rn FROM content) sc ON sc.rn = 1 + g.shot_i
LEFT JOIN (SELECT id AS cpick,   row_number() OVER () AS rn FROM crawl)   cr ON cr.rn = 1 + g.crawl_i
LEFT JOIN (SELECT t   AS tpick,  row_number() OVER () AS rn FROM pool_title) tt ON tt.rn = 1 + g.title_i;"
end

function sql_links
    set -l n (math "floor($n_caps / 3)")
    printf '%s\n' "INSERT OR IGNORE INTO capture_resource(page_capture_id, resource_capture_id)
WITH p AS (SELECT id FROM capture WHERE id % 3 = 1 ORDER BY random() LIMIT $n),
     r AS (SELECT id FROM capture WHERE id % 3 = 2 ORDER BY random() LIMIT $n)
SELECT p.id, r.id FROM p, r WHERE p.id != r.id LIMIT $n;"
end

function sql_tags
    printf '%s\n' "INSERT OR IGNORE INTO tag(name, color) VALUES
 ('Docs','blue'), ('Recipes','teal'), ('Rust','orange'), ('News','red'),
 ('API','purple'), ('To Read',''), ('Reference','green'), ('Seed','pink'),
 ('Design','yellow'), ('Blog','blue'), ('Tutorial','teal'), ('Screencast','orange'),
 ('Talk','red'), ('Podcast','purple'), ('Tooling','green'), ('Archive','pink');"
end

function sql_url_tags
    set -l nu (math "floor($n_urls * 3 / 5)")
    printf '%s\n' "INSERT OR IGNORE INTO url_tag(page_id, tag_id)
SELECT u.id, t.id
FROM (SELECT id FROM url ORDER BY random() LIMIT $nu) AS u,
     (SELECT id FROM tag ORDER BY random() LIMIT 2 + ($nu % 5)) AS t
LIMIT $nu;"
end

function sql_pending
    printf '%s\n' "WITH RECURSIVE i(n) AS (VALUES (1) UNION ALL SELECT n + 1 FROM i WHERE i.n < $n_pen),
gen(rn, st, title_i, ehex) AS MATERIALIZED (
    SELECT i.n,
           (random() & 4294967295) % 5,
           (random() & 4294967295) % 8,
           substr(hex(randomblob(6)), 1, 8)
    FROM i
)
INSERT OR IGNORE INTO pending(url, title, status, attempts, last_error, created_at)
SELECT 'https://todo-' || g.ehex || '.example.org/' || g.rn,
       CASE g.title_i WHEN 0 THEN 'Statement yearly tracker' WHEN 1 THEN 'Markdown for tables'
                      WHEN 2 THEN 'Rewrite integrations in Go' WHEN 3 THEN 'Page speed audit'
                      WHEN 4 THEN 'Draft: changelog WIP' WHEN 5 THEN 'Compare schema tools'
                      WHEN 6 THEN 'Archive old blog assets' ELSE 'Read: local-first essay' END,
       CASE WHEN g.st < 4 THEN 'queued' WHEN g.st = 4 THEN 'in_progress' ELSE 'failed' END,
       CASE WHEN g.st < 4 THEN 0 WHEN g.st = 4 THEN 1 ELSE 2 + ((random() & 4294967295) % 4) END,
       CASE WHEN g.st = 5 THEN 'seeded: fetch error ' || g.ehex END,
       unixepoch() - (g.rn * 43200)
FROM gen g;"
end

function sql_pending_tags
    set -l np (math "floor($n_pen * 2 / 3)")
    set -l nt (math "floor($n_pen / 3) + 2")
    printf '%s\n' "INSERT OR IGNORE INTO pending_tag(pending_id, tag_id)
SELECT q.id, t.id
FROM (SELECT id FROM pending ORDER BY random() LIMIT $np) AS q,
     (SELECT id FROM tag ORDER BY random() LIMIT $nt) AS t
LIMIT $np;"
end

# ── curated DEMO seed (fixed ids; INSERT OR IGNORE → fully idempotent) ──────
function sql_demo_crawl
    printf '%s\n' "INSERT OR IGNORE INTO crawl(id, name, started_at, finished_at) VALUES
 (1, 'seed: weekly news sweep', unixepoch() - 604800, unixepoch() - 602400),
 (2, 'seed: docs deep dive',    unixepoch() - 86400,  NULL),
 (3, 'seed: hot repos',         unixepoch() - 3600,   unixepoch() - 3200);"
end

function sql_demo_urls
    printf '%s\n' "INSERT OR IGNORE INTO url(id, url, scheme, host, path, query) VALUES
 (1,  'https://example.com/',                          'https', 'example.com',            '/',                          NULL),
 (2,  'https://example.com/blog/2026/local-first-apps','https', 'example.com',            '/blog/2026/local-first-apps',NULL),
 (3,  'https://example.com/blog/2026/async-rust',      'https', 'example.com',            '/blog/2026/async-rust',      NULL),
 (4,  'https://example.com/docs/getting-started',      'https', 'example.com',            '/docs/getting-started',      NULL),
 (5,  'https://sqlite.org/lang_UPSERT.html',           'https', 'sqlite.org',             '/lang_UPSERT.html',          NULL),
 (6,  'https://sqlite.org/json1.html',                 'https', 'sqlite.org',             '/json1.html',                NULL),
 (7,  'https://caniuse.com/?search=container-queries', 'https', 'caniuse.com',            '/',                          'search=container-queries'),
 (8,  'https://news.ycombinator.com/item?id=40042',    'https', 'news.ycombinator.com',   '/item',                      'id=40042'),
 (9,  'https://github.com/charmbracelet/gum',          'https', 'github.com',             '/charmbracelet/gum',         NULL),
 (10, 'https://en.wikipedia.org/wiki/Web_archiving',   'https', 'en.wikipedia.org',       '/wiki/Web_archiving',        NULL),
 (11, 'https://developer.mozilla.org/en-US/docs/Web/API','https','developer.mozilla.org', '/en-US/docs/Web/API',        NULL),
 (12, 'https://example.com/files/seed-whitepaper.pdf', 'https', 'example.com',            '/files/seed-whitepaper.pdf', NULL),
 (13, 'https://example.com/assets/app.css',            'https', 'example.com',            '/assets/app.css',            NULL),
 (14, 'https://example.com/assets/app.js',             'https', 'example.com',            '/assets/app.js',             NULL),
 (15, 'https://example.com/assets/logo.png',           'https', 'example.com',            '/assets/logo.png',           NULL),
 (16, 'https://api.example.com/v1/status',             'https', 'api.example.com',        '/v1/status',                 NULL),
 (17, 'http://www.wikipedia.org/',                     'http',  'www.wikipedia.org',      '/',                          NULL),
 (18, 'https://fishshell.com/docs/current/',           'https', 'fishshell.com',          '/docs/current/',             NULL);"
end

function sql_demo_content
    printf '%s\n' "INSERT OR IGNORE INTO
 content(id, hash, mime_type, size_original, size_stored, compression_id, data, storage_ref) VALUES
 (1,  randomblob(32), 'text/html',               84213,   28632,  1, NULL,              'blk3:seed-0001'),
 (2,  randomblob(32), 'text/html',               91221,   56210,  2, NULL,              'blk3:seed-0002'),
 (3,  randomblob(32), 'application/pdf',       1423113,  384432,  3, NULL,              'blk3:seed-0003'),
 (4,  randomblob(32), 'image/png',              305210,   30277,  3, NULL,              'blk3:seed-0004'),
 (5,  randomblob(32), 'image/png',               30277,   30277,  0, randomblob(30277), 'blk3:seed-0005'),
 (6,  randomblob(32), 'application/json',         1123,    1123,  0, randomblob(1123),  'blk3:seed-0006'),
 (7,  randomblob(32), 'text/css',                42122,   42122,  0, randomblob(42122), 'blk3:seed-0007'),
 (8,  randomblob(32), 'text/html',               68231,   22104,  1, NULL,              'blk3:seed-0008'),
 (9,  randomblob(32), 'text/html',               39421,   13402,  1, NULL,              'blk3:seed-0009'),
 (10, randomblob(32), 'text/html',               90211,   90211,  0, randomblob(90211), 'blk3:seed-0010'),
 (11, randomblob(32), 'text/html',              133922,   89871,  2, NULL,              'blk3:seed-0011'),
 (12, randomblob(32), 'text/html',               45910,   15421,  1, NULL,              'blk3:seed-0012'),
 (13, randomblob(32), 'application/javascript',  89212,   89212,  0, randomblob(89212), 'blk3:seed-0013');"
end

function sql_demo_caps
    printf '%s\n' "INSERT OR IGNORE INTO
 capture(id, page_id, content_id, captured_at, title, status_code, redirect_page_id,
         content_type, metadata, original_encoding, request_headers, response_headers,
         crawl_id, error, screenshot_content_id) VALUES
 (1,  1,  1,  unixepoch() - 2500000, 'Example Domain', 200, NULL, 'text/html; charset=utf-8',
      json_object('outlinks', 3, 'words', 96), 'gzip',
      '{\"user-agent\":\"SeedBot/1.0\",\"accept\":\"text/html,application/xhtml+xml,*/*;q=0.8\"}',
      '{\"server\":\"seed-nginx/1.25\",\"x-cache\":\"HIT\"}', 1, NULL, NULL),
 (2,  2,  2,  unixepoch() - 2000000, 'Local-first apps in 2026', 200, NULL, 'text/html; charset=utf-8',
      json_object('outlinks', 17, 'words', 1423), 'br',
      '{\"user-agent\":\"SeedBot/1.0\",\"accept\":\"text/html\"}',
      '{\"server\":\"seed-nginx/1.25\",\"x-cache\":\"MISS\"}', 1, NULL, 4),
 (3,  3,  NULL, unixepoch() - 1800000, 'Moved', 301, 4, 'text/html', NULL, 'identity',
      '{\"user-agent\":\"SeedBot/1.0\"}',
      '{\"server\":\"seed-nginx/1.25\",\"location\":\"/docs/getting-started\"}', 1, NULL, NULL),
 (4,  4,  8,  unixepoch() - 900000, 'Getting started — docs', 200, NULL, 'text/html; charset=utf-8',
      json_object('outlinks', 9, 'words', 2894), 'gzip',
      '{\"user-agent\":\"SeedBot/1.0\",\"accept\":\"text/html\"}',
      '{\"server\":\"seed-nginx/1.25\",\"x-cache\":\"MISS\"}', 2, NULL, NULL),
 (5,  5,  NULL, unixepoch() - 860400, 'UPSERT — moved', 301, 6, 'text/html', NULL, 'identity',
      '{"user-agent":"SeedBot/1.0"}',
      '{"location":"/json1.html"}', 2, NULL, NULL),
 (6,  6,  9,  unixepoch() - 860000, 'SQLite JSON1 functions', 200, NULL, 'text/html; charset=utf-8',
      json_object('outlinks', 21, 'words', 5112), 'br',
      '{"user-agent":"SeedBot/1.0"}', '{"server":"seed-nginx/1.25","x-cache":"HIT"}', 2, NULL, NULL),
 (7,  7,  10, unixepoch() - 500000, 'caniuse: container queries', 200, NULL, 'text/html; charset=utf-8',
      json_object('outlinks', 44, 'words', 2101), 'gzip',
      '{"user-agent":"SeedBot/1.0"}', '{"server":"seed-nginx/1.25","x-cache":"BYPASS"}', 2, NULL, NULL),
 (8,  8,  NULL, unixepoch() - 480000, 'Discussion #40042', 429, NULL, 'application/json',
      NULL, 'identity', '{"user-agent":"SeedBot/1.0"}',
      '{"server":"seed-proxy","retry-after":"30"}', 2, 'simulated 429 — rate limited', NULL),
 (9,  9,  11, unixepoch() - 200000, 'charmbracelet/gum', 200, NULL, 'text/html; charset=utf-8',
      json_object('outlinks', 128, 'words', 3801), 'br',
      '{"user-agent":"SeedBot/1.0"}', '{"server":"GitHub.com"}', 3, NULL, 4),
 (10, 10, 12, unixepoch() - 200000, 'Web archiving — Wikipedia', 200, NULL, 'text/html; charset=utf-8',
      json_object('outlinks', 312, 'words', 10432), 'gzip',
      '{"user-agent":"SeedBot/1.0"}', '{"server":"mw1414"}', 1, NULL, NULL),
 (11, 12, 3,  unixepoch() - 300000, 'Seed whitepaper', 200, NULL, 'application/pdf',
      NULL, 'identity', '{"user-agent":"SeedBot/1.0"}',
      '{"server":"seed-nginx/1.25","x-cache":"MISS"}', 1, NULL, NULL),
 (12, 13, 7,  unixepoch() - 300000, 'app.css', 200, NULL, 'text/css',
      NULL, 'identity', '{"user-agent":"SeedBot/1.0"}',
      '{"server":"seed-nginx/1.25","x-cache":"HIT"}', 1, NULL, NULL),
 (13, 14, 13, unixepoch() - 300000, 'app.js', 200, NULL, 'application/javascript',
      NULL, 'identity', '{"user-agent":"SeedBot/1.0"}',
      '{"server":"seed-nginx/1.25","x-cache":"HIT"}', 1, NULL, NULL),
 (14, 15, 5,  unixepoch() - 300000, 'logo.png', 200, NULL, 'image/png',
      NULL, 'identity', '{"user-agent":"SeedBot/1.0"}',
      '{"server":"seed-nginx/1.25","x-cache":"HIT"}', 1, NULL, NULL),
 (15, 16, 6,  unixepoch() - 60000, 'API status', 200, NULL, 'application/json',
      json_object('ok', json('true')), 'identity',
      '{"user-agent":"SeedBot/1.0"}', '{"server":"seed-api"}', 3, NULL, NULL),
 (16, 17, NULL, unixepoch() - 60000, 'Wikipedia portal', 500, NULL, 'text/html',
      NULL, 'identity', '{"user-agent":"SeedBot/1.0"}',
      '{"server":"seed-proxy"}', 1, 'simulated 500 — upstream unavailable', NULL),
 (17, 6,  9,  unixepoch() - 300, 'SQLite JSON1 functions (recapture)', 200, NULL, 'text/html; charset=utf-8',
      json_object('outlinks', 21, 'recapture', json('true')), 'br',
      '{"user-agent":"SeedBot/1.0"}', '{"server":"seed-nginx/1.25","x-cache":"HIT"}', 3, NULL, NULL);

INSERT OR IGNORE INTO capture_resource(page_capture_id, resource_capture_id) VALUES
 (2, 12), (2, 13), (2, 14),
 (9, 13), (9, 14),
 (10, 14),
 (4, 13), (4, 14), (4, 15),
 (6, 13);"
end

function sql_demo_tags
    printf '%s\n' "INSERT OR IGNORE INTO tag(id, name, color) VALUES
 (1, 'Docs', 'blue'), (2, 'Recipes', 'teal'), (3, 'Rust', 'orange'), (4, 'News', 'red'),
 (5, 'API', 'purple'), (6, 'To Read', NULL), (7, 'Reference', 'green'), (8, 'Seed', 'pink');"
end

function sql_demo_url_tags
    printf '%s\n' "INSERT OR IGNORE INTO url_tag(page_id, tag_id) VALUES
 (1, 4), (2, 6), (2, 8), (3, 3), (3, 6), (4, 1), (5, 1), (6, 1), (6, 7),
 (7, 7), (8, 4), (9, 8), (10, 7), (11, 1), (12, 8), (16, 5);"
end

function sql_demo_pending
    printf '%s\n' "INSERT OR IGNORE INTO pending(id, url, title, status, attempts, last_error, created_at) VALUES
 (1, 'https://hn.algolia.com/',            'Hacker News Search', 'queued',      0, NULL,                                   unixepoch() - 86400),
 (2, 'https://sqlite.org/forum/',          'SQLite Forum',       'queued',      0, NULL,                                   unixepoch() - 50000),
 (3, 'https://failing-seed.example/',      'Totally broken',     'failed',      3, 'seeded: ERR_NAME_NOT_RESOLVED',        unixepoch() - 40000),
 (4, 'https://slow-seed.example/',         'Slow site',          'in_progress', 1, NULL,                                   unixepoch() - 4000),
 (5, 'https://gum.charm.sh/',              'gum landing page',   'queued',      0, NULL,                                   unixepoch() - 700),
 (6, 'https://fishshell.com/docs/current/','fish docs',          'queued',      0, NULL,                                   unixepoch() - 300);"
end

function sql_demo_pending_tags
    printf '%s\n' "INSERT OR IGNORE INTO pending_tag(pending_id, tag_id) VALUES
 (1, 4), (2, 1), (3, 8), (5, 8), (6, 1);"
end
# ── assemble the one-shot SQL script ───────────────────────────────────────
set -l blocks "PRAGMA foreign_keys = ON;" "BEGIN;" (sql_compression)
if test "$mode" = demo
    set -a blocks (sql_demo_crawl) (sql_demo_urls) (sql_demo_content) (sql_demo_caps) \
        (sql_demo_tags) (sql_demo_url_tags) (sql_demo_pending) (sql_demo_pending_tags)
else
    contains crawls $picks; and set -a blocks (sql_crawl)
    contains urls $picks; and set -a blocks (sql_urls)
    contains content $picks; and set -a blocks (sql_content)
    contains captures $picks; and set -a blocks (sql_caps)
    contains capture_links $picks; and set -a blocks (sql_links)
    contains tags $picks; and set -a blocks (sql_tags)
    contains url_tags $picks; and set -a blocks (sql_url_tags)
    contains pending $picks; and set -a blocks (sql_pending)
    contains pending_tags $picks; and set -a blocks (sql_pending_tags)
end
set -a blocks "COMMIT;"

set -l sql_file (mktemp /tmp/seeddb.XXXXXX)
printf '%s\n' $blocks >$sql_file

if gum spin --spinner dot --title "seeding $db" -- sqlite3 -bail "$db" <$sql_file
    rm -f $sql_file
else
    echo "seed failed — replaying without spinner for the real error:" >&2
    sqlite3 -bail "$db" <$sql_file
    rm -f $sql_file
    exit 1
end

set -l fk_violations (sqlite3 "$db" "PRAGMA foreign_key_check;")
if test -n "$fk_violations"
    fail "foreign key check: $fk_violations"
end

# ── summary ────────────────────────────────────────────────────────────────
set -l db_size (du -h "$db" | cut -f1)
set -l counts (sqlite3 "$db" \
    "SELECT 'compression', count(*) FROM compression
     UNION ALL SELECT 'crawl', count(*) FROM crawl
     UNION ALL SELECT 'url', count(*) FROM url
     UNION ALL SELECT 'content', count(*) FROM content
     UNION ALL SELECT 'capture', count(*) FROM capture
     UNION ALL SELECT 'capture_resource', count(*) FROM capture_resource
     UNION ALL SELECT 'tag', count(*) FROM tag
     UNION ALL SELECT 'url_tag', count(*) FROM url_tag
     UNION ALL SELECT 'pending', count(*) FROM pending
     UNION ALL SELECT 'pending_tag', count(*) FROM pending_tag;")
gum style --border rounded --border-foreground 212 --padding "1 2" \
    "seeded $db ($db_size)" $counts

# MARKER_PART2
