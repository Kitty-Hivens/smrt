-- accounts.db: persistent user identities (from GitHub OAuth) and server-side
-- sessions keyed to a user -- the multi-user auth foundation. A sign-in is a
-- `users` row; a session id maps to a user, not to a raw token. Grants,
-- user_flags, upload moderation, and the audit log join here in later phases.

CREATE TABLE IF NOT EXISTS accounts_meta (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
    id            INTEGER PRIMARY KEY,
    github_uid    INTEGER NOT NULL UNIQUE,
    login         TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member', 'admin')),
    created_at    INTEGER NOT NULL,
    last_login_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id         TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);

-- Retire the seeded break-glass user (github_uid = 0) that the removed token
-- login opened sessions for. A DB created before that path was dropped still
-- carries the row; delete it so its sessions cascade away and nothing is left
-- pinned to the reserved uid. A no-op on fresh DBs, which never seed it.
DELETE FROM users WHERE github_uid = 0;

-- Member jar uploads awaiting moderation. A self-hosted jar never lands in the
-- shared cache directly: it stages here as `pending`, an operator approves it
-- (jar promoted to the cache) or rejects it. `note` carries the auto-gate reason
-- or the moderator's. See the upload-moderation policy.
CREATE TABLE IF NOT EXISTS mod_uploads (
    id         INTEGER PRIMARY KEY,
    uploader   INTEGER NOT NULL,
    pack_id    TEXT NOT NULL,
    filename   TEXT NOT NULL,
    sha1       TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    status     TEXT NOT NULL DEFAULT 'pending'
               CHECK (status IN ('pending', 'approved', 'rejected')),
    note       TEXT,
    created_at INTEGER NOT NULL,
    decided_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_uploads_status ON mod_uploads(status);
CREATE INDEX IF NOT EXISTS idx_uploads_uploader ON mod_uploads(uploader);

-- Rules-of-use acceptance, keyed by github uid. A member must accept before
-- authoring or forking community content. A separate table (not a users column)
-- so the idempotent CREATE-IF-NOT-EXISTS schema needs no ALTER migration.
CREATE TABLE IF NOT EXISTS terms_acceptance (
    github_uid  INTEGER PRIMARY KEY,
    accepted_at INTEGER NOT NULL
);
