//! Persistent accounts store (embedded SQLite): user identities from GitHub
//! OAuth and server-side sessions keyed to a user. The multi-user auth
//! foundation for the ladder in the multi-user issue -- a sign-in is a `users`
//! row and a session id maps to a user, not to a raw token. Same connection
//! idiom as the registry: a `Mutex<Connection>` run inside `spawn_blocking`.

use anyhow::{Context, Result};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ts_rs::TS;

const SCHEMA: &str = include_str!("schema.sql");
const SESSION_TTL: Duration = Duration::from_secs(86_400);
/// The `mod_uploads` columns `upload_from_row` reads, in its index order. One
/// source so the several upload SELECTs cannot drift from the row mapper.
const UPLOAD_COLS: &str = "id, uploader, pack_id, filename, sha1, size_bytes, status, note, created_at, \
     upstream_maintainer, decided_by";
/// Reserved uid for the synthetic machine-bearer admin (the `Bearer` token path
/// in the http layer). It is never persisted as a `users` row; the guards below
/// keep uid 0 unassignable so it can't collide with a real GitHub account.
const BREAK_GLASS_UID: i64 = 0;

/// The panel's authorization tiers, ordered low -> high: **declaration order is
/// the rank** (`Member < Admin < Debug`), so `role >= Role::Admin` is the admin
/// gate and `role >= Role::Debug` the debug gate. `Debug` is a rung ABOVE admin
/// (#39), not a flag: a guard on the compat-affecting authoring that would
/// otherwise let a casual operator corrupt the derivation graph. `member` is the
/// default on sign-in; `admin`/`debug` come from the operator allowlists.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Role {
    Member,
    Admin,
    Debug,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Member => "member",
            Role::Admin => "admin",
            Role::Debug => "debug",
        }
    }

    fn from_db(s: &str) -> Role {
        match s {
            "debug" => Role::Debug,
            "admin" => Role::Admin,
            _ => Role::Member,
        }
    }
}

/// Who is behind a request, resolved from the session's user row and attached to
/// the request by the auth middleware.
#[derive(Clone, Debug)]
pub struct Identity {
    /// GitHub numeric uid; 0 for the break-glass admin token.
    pub uid: i64,
    pub login: String,
    pub role: Role,
}

impl Identity {
    /// May this caller manage a resource owned by `owner_uid`? True for the owner
    /// themselves or for any admin-and-up role. The ownership gate for member-
    /// authored packs.
    pub fn owns_or_admin(&self, owner_uid: i64) -> bool {
        self.uid == owner_uid || self.role >= Role::Admin
    }
}

/// What somebody may do to one pack (ADR 0006). Declaration order is the rank
/// (`View < Edit < Own`), so a gate asks `level >= PackLevel::Edit` the way the
/// mirror-wide gate asks `role >= Role::Admin`.
///
/// The owner of a community namespace and an admin are never stored as a level:
/// they are what the gate knows before it reads. A stored level is only ever the
/// third answer -- somebody who is neither.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings/")]
pub enum PackLevel {
    /// Read a draft, its history and its reports.
    View,
    /// Write the config, commit, build.
    Edit,
    /// Also grant and revoke access, change visibility, delete.
    Own,
}

impl PackLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            PackLevel::View => "view",
            PackLevel::Edit => "edit",
            PackLevel::Own => "own",
        }
    }

    pub fn parse(s: &str) -> Option<PackLevel> {
        match s {
            "view" => Some(PackLevel::View),
            "edit" => Some(PackLevel::Edit),
            "own" => Some(PackLevel::Own),
            _ => None,
        }
    }
}

/// One person's access to one pack, as the access list shows it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct PackGrant {
    #[ts(type = "number")]
    pub github_uid: i64,
    /// The login as it was last seen signing in; absent for a uid granted access
    /// before its owner ever signed in here.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub login: Option<String>,
    pub level: PackLevel,
    #[ts(type = "number")]
    pub granted_by: i64,
    #[ts(type = "number")]
    pub granted_at: i64,
}

/// A registered user, for the operator's user-management view.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct UserRow {
    #[ts(type = "number")]
    pub github_uid: i64,
    pub login: String,
    pub role: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub last_login_at: i64,
}

/// A member jar upload in the moderation queue.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct UploadRow {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub uploader: i64,
    pub pack_id: String,
    pub filename: String,
    pub sha1: String,
    #[ts(type = "number")]
    pub size_bytes: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    /// Who the uploader named as the jar's upstream origin (archival provenance).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub upstream_maintainer: Option<String>,
    /// GitHub uid of the moderator who decided this upload; absent while pending.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub decided_by: Option<i64>,
}

/// One entry in the system-wide audit log: who did what, when.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct AuditRow {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub actor_uid: i64,
    pub actor_login: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
}

pub struct Accounts {
    conn: Mutex<Connection>,
}

impl Accounts {
    /// Open (creating if absent) the accounts DB at `path`, set pragmas, and
    /// apply the schema. Synchronous; called once at startup.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path)
            .with_context(|| format!("opening accounts db at {}", path.display()))?;
        Self::init(conn)
    }

    /// In-memory accounts store, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().context("opening in-memory accounts")?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .context("setting accounts pragmas")?;
        conn.execute_batch(SCHEMA)
            .context("applying accounts schema")?;
        widen_role_check(&conn).context("widening users.role check")?;
        // provenance columns on a DB that predates them (#44)
        ensure_column(&conn, "mod_uploads", "upstream_maintainer", "TEXT")?;
        ensure_column(&conn, "mod_uploads", "decided_by", "INTEGER")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Upsert the signed-in GitHub user and open a session for them, returning
    /// the opaque session id for the cookie. `forced_role` comes from the operator
    /// allowlists (admin or debug) and is authoritative -- it sets the role on the
    /// record and re-asserts it on every login. `None` seeds a plain member on
    /// first sight, then leaves the role alone so an operator's UI promotion
    /// sticks across the user's later logins. A returning user keeps their row;
    /// login and last-login refresh. Blocking; wrap in `spawn_blocking`.
    pub fn sign_in_github(
        &self,
        github_uid: i64,
        login: &str,
        forced_role: Option<Role>,
    ) -> Result<String> {
        let now = unix_now();
        let mut guard = self.conn.lock().expect("accounts mutex poisoned");
        let tx = guard.transaction().context("begin sign-in txn")?;
        match forced_role {
            Some(role) => tx.execute(
                "INSERT INTO users (github_uid, login, role, created_at, last_login_at)
                 VALUES (?1, ?2, ?4, ?3, ?3)
                 ON CONFLICT(github_uid) DO UPDATE SET
                   login = excluded.login, role = ?4, last_login_at = excluded.last_login_at",
                params![github_uid, login, now, role.as_str()],
            ),
            None => tx.execute(
                "INSERT INTO users (github_uid, login, role, created_at, last_login_at)
                 VALUES (?1, ?2, 'member', ?3, ?3)
                 ON CONFLICT(github_uid) DO UPDATE SET
                   login = excluded.login, last_login_at = excluded.last_login_at",
                params![github_uid, login, now],
            ),
        }
        .context("upsert user")?;
        let user_id: i64 = tx
            .query_row(
                "SELECT id FROM users WHERE github_uid = ?1",
                params![github_uid],
                |r| r.get(0),
            )
            .context("read user id")?;
        let sid = insert_session(&tx, user_id, now)?;
        tx.commit().context("commit sign-in")?;
        Ok(sid)
    }

    /// The identity behind a session id, if the session exists and has not
    /// expired. A lapsed session is deleted on read so the table self-prunes.
    /// Blocking; wrap in `spawn_blocking`.
    pub fn session_identity(&self, session_id: &str) -> Result<Option<Identity>> {
        let now = unix_now();
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let row = guard
            .query_row(
                "SELECT u.github_uid, u.login, u.role, s.expires_at
                 FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.id = ?1",
                params![session_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .context("read session")?;
        match row {
            Some((uid, login, role, expires_at)) if expires_at > now => Ok(Some(Identity {
                uid,
                login,
                role: Role::from_db(&role),
            })),
            Some(_) => {
                guard.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// What `github_uid` was granted on `pack_id`, or `None` when nothing was.
    ///
    /// Answers the stored third case only: the caller's gate decides ownership
    /// and the admin rung before asking. Blocking; wrap in `spawn_blocking`.
    pub fn pack_access_level(&self, pack_id: &str, github_uid: i64) -> Result<Option<PackLevel>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let level: Option<String> = guard
            .query_row(
                "SELECT level FROM pack_access WHERE pack_id = ?1 AND github_uid = ?2",
                params![pack_id, github_uid],
                |r| r.get(0),
            )
            .optional()
            .context("read pack access")?;
        Ok(level.as_deref().and_then(PackLevel::parse))
    }

    /// Grant (or move) somebody's access to a pack. Re-granting overwrites the
    /// level and re-stamps who decided it and when, so the list always says who
    /// is answerable for the access as it stands rather than as it began.
    ///
    /// Refuses the reserved uid 0, which is the synthetic break-glass identity
    /// and never a person. Blocking; wrap in `spawn_blocking`.
    pub fn grant_pack_access(
        &self,
        pack_id: &str,
        github_uid: i64,
        level: PackLevel,
        granted_by: i64,
    ) -> Result<()> {
        if github_uid == BREAK_GLASS_UID {
            anyhow::bail!("uid 0 is reserved");
        }
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        guard.execute(
            "INSERT INTO pack_access (pack_id, github_uid, level, granted_by, granted_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(pack_id, github_uid) DO UPDATE SET
                 level = excluded.level,
                 granted_by = excluded.granted_by,
                 granted_at = excluded.granted_at",
            params![pack_id, github_uid, level.as_str(), granted_by, unix_now()],
        )?;
        Ok(())
    }

    /// Take somebody's access away. `false` when they had none, so a caller can
    /// tell a revocation from a no-op. Blocking; wrap in `spawn_blocking`.
    pub fn revoke_pack_access(&self, pack_id: &str, github_uid: i64) -> Result<bool> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let gone = guard.execute(
            "DELETE FROM pack_access WHERE pack_id = ?1 AND github_uid = ?2",
            params![pack_id, github_uid],
        )?;
        Ok(gone > 0)
    }

    /// Everyone granted access to a pack, highest level first. The login is
    /// joined from `users` and is absent for a uid that has never signed in --
    /// access can be granted ahead of a first login, and the list says so rather
    /// than inventing a name. Blocking; wrap in `spawn_blocking`.
    pub fn list_pack_access(&self, pack_id: &str) -> Result<Vec<PackGrant>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let mut stmt = guard.prepare(
            "SELECT a.github_uid, u.login, a.level, a.granted_by, a.granted_at
             FROM pack_access a LEFT JOIN users u ON u.github_uid = a.github_uid
             WHERE a.pack_id = ?1
             ORDER BY CASE a.level WHEN 'own' THEN 0 WHEN 'edit' THEN 1 ELSE 2 END,
                      a.granted_at",
        )?;
        let rows = stmt
            .query_map(params![pack_id], |r| {
                Ok(PackGrant {
                    github_uid: r.get(0)?,
                    login: r.get(1)?,
                    level: r
                        .get::<_, String>(2)
                        .ok()
                        .as_deref()
                        .and_then(PackLevel::parse)
                        .unwrap_or(PackLevel::View),
                    granted_by: r.get(3)?,
                    granted_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every pack somebody was granted access to, for their own listing.
    /// Blocking; wrap in `spawn_blocking`.
    pub fn packs_granted_to(&self, github_uid: i64) -> Result<Vec<(String, PackLevel)>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let mut stmt = guard.prepare(
            "SELECT pack_id, level FROM pack_access WHERE github_uid = ?1 ORDER BY pack_id",
        )?;
        let rows = stmt
            .query_map(params![github_uid], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)
                        .ok()
                        .as_deref()
                        .and_then(PackLevel::parse)
                        .unwrap_or(PackLevel::View),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Forget a deleted pack's access list, so a pack id minted again later does
    /// not inherit who could reach the one before it.
    pub fn forget_pack_access(&self, pack_id: &str) -> Result<()> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        guard.execute(
            "DELETE FROM pack_access WHERE pack_id = ?1",
            params![pack_id],
        )?;
        Ok(())
    }

    /// Drop a session (logout). Blocking; wrap in `spawn_blocking`.
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        guard.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
        Ok(())
    }

    /// Every registered user except the reserved uid 0, newest login first.
    /// Blocking; wrap in `spawn_blocking`.
    pub fn list_users(&self) -> Result<Vec<UserRow>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let mut stmt = guard.prepare(
            "SELECT github_uid, login, role, created_at, last_login_at
             FROM users WHERE github_uid != 0
             ORDER BY last_login_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(UserRow {
                    github_uid: r.get(0)?,
                    login: r.get(1)?,
                    role: r.get(2)?,
                    created_at: r.get(3)?,
                    last_login_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Set a user's role by GitHub uid. Refuses the reserved uid 0 and an unknown
    /// role. An allowlisted uid re-promotes to admin on its next login
    /// regardless, so demoting one here only holds until they sign in again.
    /// Blocking; wrap in `spawn_blocking`.
    pub fn set_role(&self, github_uid: i64, role: &str) -> Result<()> {
        if github_uid == BREAK_GLASS_UID {
            anyhow::bail!("cannot change the reserved uid 0");
        }
        if role != "member" && role != "admin" && role != "debug" {
            anyhow::bail!("invalid role '{role}'");
        }
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let n = guard.execute(
            "UPDATE users SET role = ?2 WHERE github_uid = ?1",
            params![github_uid, role],
        )?;
        if n == 0 {
            anyhow::bail!("no user with uid {github_uid}");
        }
        Ok(())
    }

    // ── moderation queue (member jar uploads) ───────────────────────────────

    /// Enqueue a pending member upload; returns its id. `upstream_maintainer` is
    /// the uploader-named origin of the jar, kept for archival provenance.
    /// Blocking.
    pub fn enqueue_upload(
        &self,
        uploader: i64,
        pack_id: &str,
        filename: &str,
        sha1: &str,
        size_bytes: i64,
        upstream_maintainer: Option<&str>,
    ) -> Result<i64> {
        let now = unix_now();
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        guard.execute(
            "INSERT INTO mod_uploads
               (uploader, pack_id, filename, sha1, size_bytes, status, created_at, upstream_maintainer)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7)",
            params![uploader, pack_id, filename, sha1, size_bytes, now, upstream_maintainer],
        )?;
        Ok(guard.last_insert_rowid())
    }

    /// Pending uploads, oldest first -- the operator's moderation queue. Blocking.
    pub fn list_pending_uploads(&self) -> Result<Vec<UploadRow>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let mut stmt = guard.prepare(&format!(
            "SELECT {UPLOAD_COLS} FROM mod_uploads WHERE status = 'pending' ORDER BY created_at ASC"
        ))?;
        let rows = stmt
            .query_map([], upload_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// A member's own uploads (any status), newest first. Blocking.
    pub fn list_user_uploads(&self, uploader: i64) -> Result<Vec<UploadRow>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let mut stmt = guard.prepare(&format!(
            "SELECT {UPLOAD_COLS} FROM mod_uploads WHERE uploader = ?1 ORDER BY created_at DESC"
        ))?;
        let rows = stmt
            .query_map(params![uploader], upload_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// One upload by id. Blocking.
    pub fn get_upload(&self, id: i64) -> Result<Option<UploadRow>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        guard
            .query_row(
                &format!("SELECT {UPLOAD_COLS} FROM mod_uploads WHERE id = ?1"),
                params![id],
                upload_from_row,
            )
            .optional()
            .context("read upload")
    }

    /// Decide a pending upload: `approved` or `rejected`, with an optional note
    /// and the deciding moderator's uid (kept for accountability). Blocking.
    pub fn set_upload_status(
        &self,
        id: i64,
        status: &str,
        note: Option<&str>,
        decided_by: Option<i64>,
    ) -> Result<()> {
        if status != "approved" && status != "rejected" {
            anyhow::bail!("invalid upload status '{status}'");
        }
        let now = unix_now();
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let n = guard.execute(
            "UPDATE mod_uploads
               SET status = ?2, note = ?3, decided_at = ?4, decided_by = ?5
             WHERE id = ?1",
            params![id, status, note, now, decided_by],
        )?;
        if n == 0 {
            anyhow::bail!("no upload with id {id}");
        }
        Ok(())
    }

    // ── rules-of-use acceptance ─────────────────────────────────────────────

    /// Record that a user has accepted the rules of use. Idempotent. Blocking.
    pub fn accept_terms(&self, uid: i64) -> Result<()> {
        let now = unix_now();
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        guard.execute(
            "INSERT OR REPLACE INTO terms_acceptance (github_uid, accepted_at) VALUES (?1, ?2)",
            params![uid, now],
        )?;
        Ok(())
    }

    /// Whether a user has accepted the rules of use. Blocking.
    pub fn terms_accepted(&self, uid: i64) -> Result<bool> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let accepted = guard
            .query_row(
                "SELECT 1 FROM terms_acceptance WHERE github_uid = ?1",
                params![uid],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        Ok(accepted)
    }

    // ── audit log ────────────────────────────────────────────────────────────

    /// Append one entry to the system-wide audit log. Blocking. Callers treat a
    /// failure as non-fatal to the audited action -- the action already happened;
    /// a lost entry is logged, not raised.
    pub fn record_audit(
        &self,
        actor_uid: i64,
        actor_login: &str,
        action: &str,
        target: Option<&str>,
        detail: Option<&str>,
    ) -> Result<()> {
        let now = unix_now();
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        guard.execute(
            "INSERT INTO audit_log (actor_uid, actor_login, action, target, detail, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![actor_uid, actor_login, action, target, detail, now],
        )?;
        Ok(())
    }

    /// Audit entries newest first, capped at `limit`, starting after `before`.
    ///
    /// The trail only grows, so reading further back is reading older ids: the
    /// id is the sort key and the cursor at once, and a page holds still even
    /// while new entries land on top of it. Blocking.
    pub fn list_audit(&self, limit: i64, before: Option<i64>) -> Result<Vec<AuditRow>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let mut stmt = guard.prepare(
            "SELECT id, actor_uid, actor_login, action, target, detail, created_at
             FROM audit_log WHERE (?2 IS NULL OR id < ?2) ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit, before], |r| {
                Ok(AuditRow {
                    id: r.get(0)?,
                    actor_uid: r.get(1)?,
                    actor_login: r.get(2)?,
                    action: r.get(3)?,
                    target: r.get(4)?,
                    detail: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

/// Add a nullable column to an existing table when it is absent -- the ADD
/// COLUMN counterpart to `CREATE TABLE IF NOT EXISTS`, so a schema addition
/// reaches a DB that predates it. `table`/`column`/`decl` are code constants,
/// never user input, so interpolating them is safe. Idempotent.
fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    let present = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<String>>>()?
        .iter()
        .any(|name| name == column);
    if !present {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

/// Widen the `users.role` CHECK to admit the debug rung (#39). `CREATE TABLE IF
/// NOT EXISTS` cannot alter a table that predates the change, so an existing DB
/// keeps the old two-value CHECK and would reject a `debug` write. When the
/// stored DDL still lacks it, rebuild the table (SQLite's supported path for a
/// CHECK change) with ids preserved so the `sessions` foreign key stays valid.
/// Idempotent and a no-op on a fresh DB, whose schema already carries the rung.
fn widen_role_check(conn: &Connection) -> Result<()> {
    let ddl: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'users'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let needs_rebuild = ddl.is_some_and(|sql| sql.contains("CHECK") && !sql.contains("'debug'"));
    if !needs_rebuild {
        return Ok(());
    }
    // foreign_keys must be toggled outside a transaction; off so dropping the old
    // users table does not disturb sessions rows, whose ids we carry over intact.
    conn.execute_batch(
        "PRAGMA foreign_keys = OFF;
         BEGIN;
         CREATE TABLE users_new (
             id            INTEGER PRIMARY KEY,
             github_uid    INTEGER NOT NULL UNIQUE,
             login         TEXT NOT NULL,
             role          TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member', 'admin', 'debug')),
             created_at    INTEGER NOT NULL,
             last_login_at INTEGER NOT NULL
         );
         INSERT INTO users_new (id, github_uid, login, role, created_at, last_login_at)
           SELECT id, github_uid, login, role, created_at, last_login_at FROM users;
         DROP TABLE users;
         ALTER TABLE users_new RENAME TO users;
         COMMIT;
         PRAGMA foreign_keys = ON;",
    )?;
    Ok(())
}

fn upload_from_row(r: &rusqlite::Row) -> rusqlite::Result<UploadRow> {
    Ok(UploadRow {
        id: r.get(0)?,
        uploader: r.get(1)?,
        pack_id: r.get(2)?,
        filename: r.get(3)?,
        sha1: r.get(4)?,
        size_bytes: r.get(5)?,
        status: r.get(6)?,
        note: r.get(7)?,
        created_at: r.get(8)?,
        upstream_maintainer: r.get(9)?,
        decided_by: r.get(10)?,
    })
}

fn insert_session(conn: &Connection, user_id: i64, now: i64) -> Result<String> {
    let sid = random_token();
    let expires = now + SESSION_TTL.as_secs() as i64;
    conn.execute(
        "INSERT INTO sessions (id, user_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![sid, user_id, now, expires],
    )
    .context("insert session")?;
    Ok(sid)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A 256-bit random token, hex-encoded: opaque session ids and OAuth `state`
/// nonces. Sourced from the OS CSPRNG so it is unguessable.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_is_granted_moved_and_taken_away() {
        let a = Accounts::open_in_memory().unwrap();
        assert_eq!(a.pack_access_level("Create", 42).unwrap(), None);

        a.grant_pack_access("Create", 42, PackLevel::View, 1)
            .unwrap();
        assert_eq!(
            a.pack_access_level("Create", 42).unwrap(),
            Some(PackLevel::View)
        );

        // re-granting moves the level rather than adding a second row, and
        // re-stamps who is answerable for the access as it stands
        a.grant_pack_access("Create", 42, PackLevel::Edit, 7)
            .unwrap();
        assert_eq!(
            a.pack_access_level("Create", 42).unwrap(),
            Some(PackLevel::Edit)
        );
        let list = a.list_pack_access("Create").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].granted_by, 7);

        assert!(a.revoke_pack_access("Create", 42).unwrap());
        assert!(
            !a.revoke_pack_access("Create", 42).unwrap(),
            "second revoke is a no-op"
        );
        assert_eq!(a.pack_access_level("Create", 42).unwrap(), None);
    }

    #[test]
    fn access_is_scoped_to_one_pack_and_names_who_it_can() {
        let a = Accounts::open_in_memory().unwrap();
        a.sign_in_github(42, "octocat", None).unwrap();
        a.grant_pack_access("Create", 42, PackLevel::Edit, 1)
            .unwrap();
        a.grant_pack_access("Create", 99, PackLevel::Own, 1)
            .unwrap();

        // a grant on one pack says nothing about another
        assert_eq!(a.pack_access_level("Industrial", 42).unwrap(), None);
        assert_eq!(
            a.packs_granted_to(42).unwrap(),
            vec![("Create".to_string(), PackLevel::Edit)]
        );

        // highest level first; a uid that never signed in has no login to show
        let list = a.list_pack_access("Create").unwrap();
        assert_eq!(list[0].github_uid, 99);
        assert_eq!(list[0].login, None);
        assert_eq!(list[1].login.as_deref(), Some("octocat"));
    }

    #[test]
    fn a_deleted_pack_does_not_bequeath_its_access_list() {
        // ids are re-mintable: a new pack under an old name must not inherit
        // whoever could reach the one before it
        let a = Accounts::open_in_memory().unwrap();
        a.grant_pack_access("Create", 42, PackLevel::Edit, 1)
            .unwrap();
        a.forget_pack_access("Create").unwrap();
        assert_eq!(a.pack_access_level("Create", 42).unwrap(), None);
    }

    #[test]
    fn the_reserved_uid_is_not_a_person_to_grant_to() {
        let a = Accounts::open_in_memory().unwrap();
        assert!(a.grant_pack_access("Create", 0, PackLevel::Own, 1).is_err());
    }

    #[test]
    fn github_sign_in_persists_user_and_resolves_session() {
        let a = Accounts::open_in_memory().unwrap();
        let sid = a.sign_in_github(42, "octocat", None).unwrap();
        let id = a.session_identity(&sid).unwrap().expect("session resolves");
        assert_eq!(id.uid, 42);
        assert_eq!(id.login, "octocat");
        assert_eq!(id.role, Role::Member);

        // a second sign-in reuses the row (no duplicate), and the allowlist can
        // promote the same uid to admin
        let sid2 = a
            .sign_in_github(42, "octocat-renamed", Some(Role::Admin))
            .unwrap();
        let id2 = a.session_identity(&sid2).unwrap().unwrap();
        assert_eq!(id2.login, "octocat-renamed");
        assert_eq!(id2.role, Role::Admin);
        let users: i64 = a
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM users WHERE github_uid = 42",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(users, 1);
    }

    #[test]
    fn deleted_session_stops_resolving() {
        let a = Accounts::open_in_memory().unwrap();
        let sid = a.sign_in_github(7, "x", Some(Role::Admin)).unwrap();
        a.delete_session(&sid).unwrap();
        assert!(a.session_identity(&sid).unwrap().is_none());
    }

    #[test]
    fn upload_keeps_provenance_and_the_decider() {
        let a = Accounts::open_in_memory().unwrap();
        let id = a
            .enqueue_upload(
                42,
                "u/42/p",
                "thaumcraft.jar",
                &"a".repeat(40),
                100,
                Some("the upstream"),
            )
            .unwrap();
        let row = a.get_upload(id).unwrap().unwrap();
        assert_eq!(row.status, "pending");
        assert_eq!(row.upstream_maintainer.as_deref(), Some("the upstream"));
        assert_eq!(row.decided_by, None); // undecided while pending

        a.set_upload_status(id, "approved", None, Some(7)).unwrap();
        let row = a.get_upload(id).unwrap().unwrap();
        assert_eq!(row.status, "approved");
        assert_eq!(row.decided_by, Some(7));
        // the origin the uploader named survives the decision
        assert_eq!(row.upstream_maintainer.as_deref(), Some("the upstream"));
    }

    #[test]
    fn ensure_column_adds_missing_and_is_idempotent() {
        // a mod_uploads created before the provenance columns
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE mod_uploads (
                 id INTEGER PRIMARY KEY, uploader INTEGER NOT NULL, pack_id TEXT NOT NULL,
                 filename TEXT NOT NULL, sha1 TEXT NOT NULL, size_bytes INTEGER NOT NULL,
                 status TEXT NOT NULL DEFAULT 'pending', note TEXT, created_at INTEGER NOT NULL,
                 decided_at INTEGER);",
        )
        .unwrap();
        ensure_column(&conn, "mod_uploads", "upstream_maintainer", "TEXT").unwrap();
        ensure_column(&conn, "mod_uploads", "decided_by", "INTEGER").unwrap();
        // a second run is a no-op (column already there)
        ensure_column(&conn, "mod_uploads", "decided_by", "INTEGER").unwrap();
        conn.execute(
            "INSERT INTO mod_uploads
               (uploader, pack_id, filename, sha1, size_bytes, status, created_at,
                upstream_maintainer, decided_by)
             VALUES (1, 'p', 'f.jar', 'sha', 1, 'pending', 0, 'up', 9)",
            [],
        )
        .unwrap();
        let (m, d): (String, i64) = conn
            .query_row(
                "SELECT upstream_maintainer, decided_by FROM mod_uploads",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(m, "up");
        assert_eq!(d, 9);
    }

    #[test]
    fn debug_rung_outranks_admin_and_is_grantable() {
        let a = Accounts::open_in_memory().unwrap();
        // allowlisted as debug -> the top rung, re-asserted on login
        let sid = a.sign_in_github(3, "t", Some(Role::Debug)).unwrap();
        let id = a.session_identity(&sid).unwrap().unwrap();
        assert_eq!(id.role, Role::Debug);
        assert!(id.role > Role::Admin);

        // a member can be granted debug through the UI path
        a.sign_in_github(11, "helper", None).unwrap();
        a.set_role(11, "debug").unwrap();
        let sid2 = a.sign_in_github(11, "helper", None).unwrap();
        assert_eq!(
            a.session_identity(&sid2).unwrap().unwrap().role,
            Role::Debug
        );

        // an unknown role is still rejected
        assert!(a.set_role(11, "root").is_err());
    }

    #[test]
    fn ui_promotion_sticks_but_allowlist_stays_authoritative() {
        let a = Accounts::open_in_memory().unwrap();
        // a non-allowlisted member is promoted via the UI, then signs in again
        a.sign_in_github(5, "m", None).unwrap();
        a.set_role(5, "admin").unwrap();
        let sid = a.sign_in_github(5, "m", None).unwrap();
        assert_eq!(a.session_identity(&sid).unwrap().unwrap().role, Role::Admin);

        // an allowlisted user re-promotes to admin on login even after a demote
        a.sign_in_github(9, "op", Some(Role::Admin)).unwrap();
        a.set_role(9, "member").unwrap();
        let sid2 = a.sign_in_github(9, "op", Some(Role::Admin)).unwrap();
        assert_eq!(
            a.session_identity(&sid2).unwrap().unwrap().role,
            Role::Admin
        );

        // list excludes the reserved uid 0
        let users = a.list_users().unwrap();
        assert_eq!(users.len(), 2);
        assert!(users.iter().all(|u| u.github_uid != 0));

        // the reserved uid 0 is untouchable
        assert!(a.set_role(0, "member").is_err());
    }

    #[test]
    fn role_check_migration_admits_debug_on_a_legacy_db() {
        // a DB created before the debug rung: the two-value CHECK rejects 'debug'
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                 id INTEGER PRIMARY KEY, github_uid INTEGER NOT NULL UNIQUE, login TEXT NOT NULL,
                 role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member', 'admin')),
                 created_at INTEGER NOT NULL, last_login_at INTEGER NOT NULL);
             CREATE TABLE sessions (id TEXT PRIMARY KEY,
                 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
             INSERT INTO users (github_uid, login, role, created_at, last_login_at)
               VALUES (5, 'x', 'admin', 0, 0);
             INSERT INTO sessions VALUES ('s', 1, 0, 0);",
        )
        .unwrap();
        assert!(
            conn.execute("UPDATE users SET role = 'debug' WHERE github_uid = 5", [])
                .is_err(),
            "legacy CHECK should reject debug"
        );

        widen_role_check(&conn).unwrap();

        // after the rebuild debug is accepted and the row + its session survived
        conn.execute("UPDATE users SET role = 'debug' WHERE github_uid = 5", [])
            .unwrap();
        let role: String = conn
            .query_row("SELECT role FROM users WHERE github_uid = 5", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(role, "debug");
        let sessions: i64 = conn
            .query_row("SELECT count(*) FROM sessions WHERE user_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(sessions, 1);

        // second run is a no-op (DDL already carries the rung)
        widen_role_check(&conn).unwrap();
    }

    #[test]
    fn audit_log_records_and_lists_newest_first() {
        let a = Accounts::open_in_memory().unwrap();
        a.record_audit(42, "octocat", "role.set", Some("7"), Some("admin"))
            .unwrap();
        a.record_audit(42, "octocat", "upload.approve", Some("deadbeef"), None)
            .unwrap();
        let rows = a.list_audit(10, None).unwrap();
        assert_eq!(rows.len(), 2);
        // newest first
        assert_eq!(rows[0].action, "upload.approve");
        assert_eq!(rows[0].target.as_deref(), Some("deadbeef"));
        assert_eq!(rows[0].detail, None);
        assert_eq!(rows[1].action, "role.set");
        assert_eq!(rows[1].actor_login, "octocat");
        assert_eq!(rows[1].detail.as_deref(), Some("admin"));
    }

    // The trail is read backwards a page at a time, and an entry landing while
    // someone reads must not shift the page under them -- which is why the
    // cursor is an id rather than an offset.
    #[test]
    fn the_trail_reads_back_a_page_at_a_time() {
        let a = Accounts::open_in_memory().unwrap();
        for i in 0..5 {
            a.record_audit(42, "octocat", &format!("act.{i}"), None, None)
                .unwrap();
        }
        let first = a.list_audit(2, None).unwrap();
        assert_eq!(
            first.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(),
            ["act.4", "act.3"]
        );

        a.record_audit(42, "octocat", "act.new", None, None)
            .unwrap();

        let second = a.list_audit(2, Some(first[1].id)).unwrap();
        assert_eq!(
            second.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(),
            ["act.2", "act.1"],
            "the newcomer lands on top of the trail, not into the page being read"
        );
        let third = a.list_audit(2, Some(second[1].id)).unwrap();
        assert_eq!(
            third.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(),
            ["act.0"]
        );
        assert!(a.list_audit(2, Some(third[0].id)).unwrap().is_empty());
    }
}
