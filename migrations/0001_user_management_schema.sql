-- User management schema (SQLite >= 3.42, STRICT tables).
-- Adapted from user-schema-2.sql for sqlx::migrate!:
--   * PRAGMAs removed  -> set per-connection in src/db.rs (SqliteConnectOptions)
--   * No BEGIN/COMMIT  -> sqlx wraps each migration in a transaction

-- -----------------------------------------------------------------------------
-- 1. USERS  (identity + account state only; no secrets here)
-- -----------------------------------------------------------------------------
CREATE TABLE users (
    id                  INTEGER PRIMARY KEY,
    public_id           TEXT    NOT NULL UNIQUE DEFAULT (
                            lower(hex(randomblob(4))) || '-' ||
                            lower(hex(randomblob(2))) || '-4' ||
                            substr(lower(hex(randomblob(2))), 2) || '-' ||
                            substr('89ab', abs(random()) % 4 + 1, 1) ||
                            substr(lower(hex(randomblob(2))), 2) || '-' ||
                            lower(hex(randomblob(6)))
                        ),

    email               TEXT    COLLATE NOCASE,
    email_verified_at   INTEGER,
    username            TEXT    COLLATE NOCASE,
    phone_e164          TEXT,                       -- e.g. +14155552671
    phone_verified_at   INTEGER,

    status              TEXT    NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending','active','suspended','deactivated')),
                        -- pending: signed up but not yet verified or approved.
                        -- active: normal.
                        -- suspended: blocked by an admin or by automated abuse detection, usually temporarily.
                        -- deactivated: closed, typically by the user, and often reversible.
    status_reason       TEXT, -- Example: 'chargeback', 'spam reports', 'user requested closure'
    is_system           INTEGER NOT NULL DEFAULT 0 CHECK (is_system IN (0,1)), -- bots / service accounts
    last_login_at       INTEGER,
    last_login_ip       TEXT,
    login_count         INTEGER NOT NULL DEFAULT 0 CHECK (login_count >= 0),

    terms_accepted_at   INTEGER,
    terms_version       TEXT,

    created_at          INTEGER    NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    updated_at          INTEGER    NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    deleted_at          INTEGER,                       -- soft delete / tombstone

    CHECK (email IS NOT NULL OR username IS NOT NULL OR phone_e164 IS NOT NULL),
    CHECK (email IS NULL OR (length(email) BETWEEN 3 AND 254 AND email LIKE '_%@_%._%')),
    CHECK (username IS NULL OR (length(username) BETWEEN 3 AND 32
                                AND username NOT GLOB '*[^A-Za-z0-9_.-]*')),
    CHECK (phone_e164 IS NULL OR phone_e164 GLOB '+[1-9]*'),
    CHECK (email_verified_at IS NULL OR email IS NOT NULL)
) STRICT;

-- Uniqueness applies to non-deleted users only, so identifiers can be reused
-- after an account is deleted. (NULLs never collide in SQLite unique indexes.)
CREATE UNIQUE INDEX ux_users_email    ON users(email)      WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX ux_users_username ON users(username)   WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX ux_users_phone    ON users(phone_e164) WHERE deleted_at IS NULL;
CREATE INDEX ix_users_status          ON users(status)     WHERE deleted_at IS NULL;
CREATE INDEX ix_users_created_at      ON users(created_at);
CREATE INDEX ix_users_deleted_at      ON users(deleted_at) WHERE deleted_at IS NOT NULL;

-- -----------------------------------------------------------------------------
-- 2. CREDENTIALS  (separate table: keeps hashes out of SELECT * on users, and
--    lets SSO-only / passwordless users simply have no row)
-- -----------------------------------------------------------------------------
CREATE TABLE user_credentials (
    user_id             INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    password_hash       TEXT    NOT NULL,           -- full PHC string incl. salt & params
    password_algo       TEXT    NOT NULL DEFAULT 'argon2id',
    password_changed_at INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    must_change         INTEGER NOT NULL DEFAULT 0 CHECK (must_change IN (0,1)),
    failed_attempts     INTEGER NOT NULL DEFAULT 0 CHECK (failed_attempts >= 0),
    last_failed_at      INTEGER,
    locked_until        INTEGER                         -- brute-force lockout
) STRICT;

-- Prevent password reuse (store hashes of previous passwords)
CREATE TABLE password_history (
    id            INTEGER PRIMARY KEY,
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    password_hash TEXT    NOT NULL,
    created_at    INTEGER    NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER))
) STRICT;
CREATE INDEX ix_password_history_user ON password_history(user_id, created_at DESC);

-- -----------------------------------------------------------------------------
-- 3. PROFILES  (non-auth attributes; extend freely per project)
-- -----------------------------------------------------------------------------
CREATE TABLE user_profiles (
    user_id        INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    display_name   TEXT,
    first_name     TEXT,
    last_name      TEXT,
    avatar_url     TEXT,
    bio            TEXT,
    birth_date     TEXT CHECK (birth_date IS NULL OR birth_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    locale         TEXT NOT NULL DEFAULT 'en',      -- BCP-47, e.g. en-US
    timezone       TEXT NOT NULL DEFAULT 'UTC',     -- IANA, e.g. Asia/Kolkata
    metadata       TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),  -- project-specific extras
    created_at     INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    updated_at     INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER))
) STRICT;

-- -----------------------------------------------------------------------------
-- 4. RBAC  (roles -> permissions, users -> roles)
-- -----------------------------------------------------------------------------
CREATE TABLE roles (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE COLLATE NOCASE
                 CHECK (name NOT GLOB '*[^a-z0-9_:.-]*'),   -- lowercase slug
    description  TEXT,
    is_system    INTEGER NOT NULL DEFAULT 0 CHECK (is_system IN (0,1)),  -- protected from deletion
    created_at   INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    updated_at   INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER))
) STRICT;

CREATE TABLE permissions (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE COLLATE NOCASE
                 CHECK (name NOT GLOB '*[^a-z0-9_:.*-]*'),  -- e.g. users:read, billing:*
    description  TEXT,
    created_at   INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER))
) STRICT;

CREATE TABLE role_permissions (
    role_id        INTEGER NOT NULL REFERENCES roles(id)       ON DELETE CASCADE,
    permission_id  INTEGER NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
    PRIMARY KEY (role_id, permission_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX ix_role_permissions_perm ON role_permissions(permission_id);

CREATE TABLE user_roles (
    user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id      INTEGER NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    granted_by   INTEGER REFERENCES users(id) ON DELETE SET NULL,
    granted_at   INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    expires_at   INTEGER,                                -- optional temporary grants
    PRIMARY KEY (user_id, role_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX ix_user_roles_role ON user_roles(role_id);

-- -----------------------------------------------------------------------------
-- 5. SESSIONS  (server-side sessions or refresh tokens; store hash only)
-- -----------------------------------------------------------------------------
CREATE TABLE sessions (
    id                  INTEGER PRIMARY KEY,
    public_id           TEXT NOT NULL UNIQUE DEFAULT (lower(hex(randomblob(16)))),
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash          TEXT NOT NULL UNIQUE,        -- SHA-256 hex of the opaque token
    family_id           TEXT,                        -- refresh-token rotation family (reuse detection)
    replaced_by         INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
    ip_address          TEXT,
    user_agent          TEXT,
    device_name         TEXT,
    created_at          INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    last_used_at        INTEGER,
    expires_at          INTEGER NOT NULL,
    revoked_at          INTEGER,
    revoked_reason      TEXT
) STRICT;
CREATE INDEX ix_sessions_user_active ON sessions(user_id, expires_at) WHERE revoked_at IS NULL;
CREATE INDEX ix_sessions_expires     ON sessions(expires_at);
CREATE INDEX ix_sessions_family      ON sessions(family_id) WHERE family_id IS NOT NULL;

-- -----------------------------------------------------------------------------
-- 6. ONE-TIME TOKENS  (email verification, password reset, magic link, invite)
-- -----------------------------------------------------------------------------
CREATE TABLE auth_tokens (
    id           INTEGER PRIMARY KEY,
    user_id      INTEGER REFERENCES users(id) ON DELETE CASCADE,  -- NULL for invites to non-users
    purpose      TEXT NOT NULL CHECK (purpose IN
                   ('verify_email','verify_phone','reset_password','magic_link','invite','change_email')),
    token_hash   TEXT NOT NULL UNIQUE,
    target       TEXT,                                -- email/phone the token was sent to
    metadata     TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
    created_at   INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    expires_at   INTEGER NOT NULL,
    used_at      INTEGER,
    CHECK (user_id IS NOT NULL OR purpose = 'invite')
) STRICT;
CREATE INDEX ix_auth_tokens_user_purpose ON auth_tokens(user_id, purpose) WHERE used_at IS NULL;
CREATE INDEX ix_auth_tokens_expires      ON auth_tokens(expires_at);

-- -----------------------------------------------------------------------------
-- 7. OAUTH / SSO IDENTITIES
-- -----------------------------------------------------------------------------
CREATE TABLE oauth_accounts (
    id                INTEGER PRIMARY KEY,
    user_id           INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider          TEXT NOT NULL COLLATE NOCASE,   -- google, github, microsoft, okta...
    provider_user_id  TEXT NOT NULL,
    provider_email    TEXT COLLATE NOCASE,
    access_token_enc  BLOB,                           -- ENCRYPT at app level if you store these
    refresh_token_enc BLOB,
    token_expires_at  INTEGER,
    raw_profile       TEXT CHECK (raw_profile IS NULL OR json_valid(raw_profile)),
    created_at        INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    updated_at        INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    UNIQUE (provider, provider_user_id)
) STRICT;
CREATE INDEX ix_oauth_user ON oauth_accounts(user_id);

-- -----------------------------------------------------------------------------
-- 8. MULTI-FACTOR AUTH
-- -----------------------------------------------------------------------------
CREATE TABLE mfa_factors (
    id            INTEGER PRIMARY KEY,
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    type          TEXT NOT NULL CHECK (type IN ('totp','sms','email','webauthn')),
    label         TEXT,
    secret_enc    BLOB,                               -- ENCRYPTED TOTP secret / WebAuthn public key
    credential_id TEXT,                               -- WebAuthn credential id
    sign_count    INTEGER NOT NULL DEFAULT 0,
    is_verified   INTEGER NOT NULL DEFAULT 0 CHECK (is_verified IN (0,1)),
    created_at    INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    last_used_at  INTEGER
) STRICT;
CREATE INDEX ix_mfa_user ON mfa_factors(user_id);
CREATE UNIQUE INDEX ux_mfa_credential ON mfa_factors(credential_id) WHERE credential_id IS NOT NULL;

CREATE TABLE mfa_recovery_codes (
    id          INTEGER PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash   TEXT NOT NULL,
    used_at     INTEGER,
    created_at  INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    UNIQUE (user_id, code_hash)
) STRICT;

-- -----------------------------------------------------------------------------
-- 9. API KEYS  (personal access tokens / service keys)
-- -----------------------------------------------------------------------------
CREATE TABLE api_keys (
    id            INTEGER PRIMARY KEY,
    public_id     TEXT NOT NULL UNIQUE DEFAULT (lower(hex(randomblob(16)))),
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    key_prefix    TEXT NOT NULL,                      -- first ~8 chars, shown in UI for identification
    key_hash      TEXT NOT NULL UNIQUE,               -- SHA-256 hex of full key
    scopes        TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(scopes)),  -- JSON array of permission names
    created_at    INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER)),
    last_used_at  INTEGER,
    last_used_ip  TEXT,
    expires_at    INTEGER,
    revoked_at    INTEGER
) STRICT;
CREATE INDEX ix_api_keys_user ON api_keys(user_id) WHERE revoked_at IS NULL;

-- -----------------------------------------------------------------------------
-- 10. SECURITY / AUDIT LOGS  (append-only; survive user deletion)
-- -----------------------------------------------------------------------------
CREATE TABLE login_attempts (
    id           INTEGER PRIMARY KEY,
    identifier   TEXT COLLATE NOCASE,                 -- email/username attempted (may not exist)
    user_id      INTEGER REFERENCES users(id) ON DELETE SET NULL,
    ip_address   TEXT,
    user_agent   TEXT,
    success      INTEGER NOT NULL CHECK (success IN (0,1)),
    failure_reason TEXT,
    created_at   INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER))
) STRICT;
CREATE INDEX ix_login_attempts_ident_time ON login_attempts(identifier, created_at DESC);
CREATE INDEX ix_login_attempts_ip_time    ON login_attempts(ip_address, created_at DESC);

CREATE TABLE audit_log (
    id             INTEGER PRIMARY KEY,
    actor_user_id  INTEGER REFERENCES users(id) ON DELETE SET NULL,
    target_user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    action         TEXT NOT NULL,                     -- e.g. user.created, role.granted, password.changed
    entity_type    TEXT,
    entity_id      TEXT,
    ip_address     TEXT,
    user_agent     TEXT,
    details        TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(details)),
    created_at     INTEGER NOT NULL DEFAULT (CAST(round(unixepoch('subsec') * 1000) AS INTEGER))
) STRICT;
CREATE INDEX ix_audit_target  ON audit_log(target_user_id, created_at DESC);
CREATE INDEX ix_audit_actor   ON audit_log(actor_user_id, created_at DESC);
CREATE INDEX ix_audit_action  ON audit_log(action, created_at DESC);

-- Make the audit log tamper-resistant at the DB level
CREATE TRIGGER trg_audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
CREATE TRIGGER trg_audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;

-- -----------------------------------------------------------------------------
-- 11. TRIGGERS: auto-maintain updated_at
--     (the WHEN clause prevents overwriting an explicit updated_at)
-- -----------------------------------------------------------------------------
CREATE TRIGGER trg_users_updated_at AFTER UPDATE ON users
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE users SET updated_at = CAST(round(unixepoch('subsec') * 1000) AS INTEGER) WHERE id = NEW.id;
END;

CREATE TRIGGER trg_profiles_updated_at AFTER UPDATE ON user_profiles
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE user_profiles SET updated_at = CAST(round(unixepoch('subsec') * 1000) AS INTEGER) WHERE user_id = NEW.user_id;
END;

CREATE TRIGGER trg_roles_updated_at AFTER UPDATE ON roles
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE roles SET updated_at = CAST(round(unixepoch('subsec') * 1000) AS INTEGER) WHERE id = NEW.id;
END;

CREATE TRIGGER trg_oauth_updated_at AFTER UPDATE ON oauth_accounts
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE oauth_accounts SET updated_at = CAST(round(unixepoch('subsec') * 1000) AS INTEGER) WHERE id = NEW.id;
END;

-- Protect built-in roles from deletion
CREATE TRIGGER trg_roles_protect_system BEFORE DELETE ON roles
FOR EACH ROW WHEN OLD.is_system = 1
BEGIN SELECT RAISE(ABORT, 'cannot delete a system role'); END;

-- Auto-create an empty profile for every new user
CREATE TRIGGER trg_users_create_profile AFTER INSERT ON users
BEGIN
    INSERT INTO user_profiles (user_id) VALUES (NEW.id);
END;

-- When a user is deactivated/suspended/soft-deleted, revoke all live sessions
CREATE TRIGGER trg_users_revoke_sessions AFTER UPDATE OF status, deleted_at ON users
FOR EACH ROW WHEN (NEW.status IN ('suspended','deactivated') AND OLD.status <> NEW.status)
                OR (NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL)
BEGIN
    UPDATE sessions
       SET revoked_at = CAST(round(unixepoch('subsec') * 1000) AS INTEGER),
           revoked_reason = 'account_status_change'
     WHERE user_id = NEW.id AND revoked_at IS NULL;
END;

-- -----------------------------------------------------------------------------
-- 12. VIEWS
-- -----------------------------------------------------------------------------
-- Live (non-deleted) users with profile; never exposes credentials
CREATE VIEW v_users AS
SELECT u.id, u.public_id, u.email, u.email_verified_at, u.username, u.phone_e164,
       u.status, u.last_login_at, u.created_at, u.updated_at,
       p.display_name, p.first_name, p.last_name, p.avatar_url, p.locale, p.timezone
  FROM users u
  LEFT JOIN user_profiles p ON p.user_id = u.id
 WHERE u.deleted_at IS NULL;

-- Flattened permissions per user (respects role expiry)
CREATE VIEW v_user_permissions AS
SELECT DISTINCT ur.user_id, p.name AS permission
  FROM user_roles ur
  JOIN role_permissions rp ON rp.role_id = ur.role_id
  JOIN permissions p       ON p.id = rp.permission_id
 WHERE ur.expires_at IS NULL OR ur.expires_at > CAST(round(unixepoch('subsec') * 1000) AS INTEGER);

CREATE VIEW v_user_roles AS
SELECT ur.user_id, r.name AS role
  FROM user_roles ur
  JOIN roles r ON r.id = ur.role_id
 WHERE ur.expires_at IS NULL OR ur.expires_at > CAST(round(unixepoch('subsec') * 1000) AS INTEGER);

-- Currently valid sessions
CREATE VIEW v_active_sessions AS
SELECT * FROM sessions
 WHERE revoked_at IS NULL AND expires_at > CAST(round(unixepoch('subsec') * 1000) AS INTEGER);

