//! Persistent accounts store (embedded SQLite): user identities from GitHub
//! OAuth and server-side sessions keyed to a user. The multi-user auth
//! foundation for the ladder in the multi-user issue -- a sign-in is a `users`
//! row and a session id maps to a user, not to a raw token. Same connection
//! idiom as the registry: a `Mutex<Connection>` run inside `spawn_blocking`.

use anyhow::{Context, Result};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ts_rs::TS;

const SCHEMA: &str = include_str!("schema.sql");
const SESSION_TTL: Duration = Duration::from_secs(86_400);
/// Reserved uid for the synthetic machine-bearer admin (the `Bearer` token path
/// in the http layer). It is never persisted as a `users` row; the guards below
/// keep uid 0 unassignable so it can't collide with a real GitHub account.
const BREAK_GLASS_UID: i64 = 0;

/// The panel's authorization tiers, ordered low -> high: **declaration order is
/// the rank** (`Member < Admin`), so `role >= Role::Admin` is the admin gate and
/// the future `Debug` rung -- a role ABOVE admin (#39), not a flag -- slots on
/// top for free by being declared after `Admin`. `member` is the default on
/// sign-in; `admin` comes from the operator allowlist.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Role {
    Member,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Member => "member",
            Role::Admin => "admin",
        }
    }

    fn from_db(s: &str) -> Role {
        match s {
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
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Upsert the signed-in GitHub user and open a session for them, returning
    /// the opaque session id for the cookie. `is_admin` comes from the operator
    /// allowlist and sets the role on the record -- the admin source until DB
    /// grants land. A returning user keeps their row; login and last-login
    /// refresh. Blocking; wrap in `spawn_blocking`.
    pub fn sign_in_github(&self, github_uid: i64, login: &str, is_admin: bool) -> Result<String> {
        let now = unix_now();
        // Allowlisted uids: env is authoritative, always admin. Others: seed
        // member on first sight, then leave the role alone so an operator's UI
        // promotion sticks across the user's later logins.
        let sql = if is_admin {
            "INSERT INTO users (github_uid, login, role, created_at, last_login_at)
             VALUES (?1, ?2, 'admin', ?3, ?3)
             ON CONFLICT(github_uid) DO UPDATE SET
               login = excluded.login, role = 'admin', last_login_at = excluded.last_login_at"
        } else {
            "INSERT INTO users (github_uid, login, role, created_at, last_login_at)
             VALUES (?1, ?2, 'member', ?3, ?3)
             ON CONFLICT(github_uid) DO UPDATE SET
               login = excluded.login, last_login_at = excluded.last_login_at"
        };
        let mut guard = self.conn.lock().expect("accounts mutex poisoned");
        let tx = guard.transaction().context("begin sign-in txn")?;
        tx.execute(sql, params![github_uid, login, now])
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
        if role != "member" && role != "admin" {
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

    /// Enqueue a pending member upload; returns its id. Blocking.
    pub fn enqueue_upload(
        &self,
        uploader: i64,
        pack_id: &str,
        filename: &str,
        sha1: &str,
        size_bytes: i64,
    ) -> Result<i64> {
        let now = unix_now();
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        guard.execute(
            "INSERT INTO mod_uploads
               (uploader, pack_id, filename, sha1, size_bytes, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
            params![uploader, pack_id, filename, sha1, size_bytes, now],
        )?;
        Ok(guard.last_insert_rowid())
    }

    /// Pending uploads, oldest first -- the operator's moderation queue. Blocking.
    pub fn list_pending_uploads(&self) -> Result<Vec<UploadRow>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let mut stmt = guard.prepare(
            "SELECT id, uploader, pack_id, filename, sha1, size_bytes, status, note, created_at
             FROM mod_uploads WHERE status = 'pending' ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([], upload_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// A member's own uploads (any status), newest first. Blocking.
    pub fn list_user_uploads(&self, uploader: i64) -> Result<Vec<UploadRow>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let mut stmt = guard.prepare(
            "SELECT id, uploader, pack_id, filename, sha1, size_bytes, status, note, created_at
             FROM mod_uploads WHERE uploader = ?1 ORDER BY created_at DESC",
        )?;
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
                "SELECT id, uploader, pack_id, filename, sha1, size_bytes, status, note, created_at
                 FROM mod_uploads WHERE id = ?1",
                params![id],
                upload_from_row,
            )
            .optional()
            .context("read upload")
    }

    /// Decide a pending upload: `approved` or `rejected`, with an optional note.
    /// Blocking.
    pub fn set_upload_status(&self, id: i64, status: &str, note: Option<&str>) -> Result<()> {
        if status != "approved" && status != "rejected" {
            anyhow::bail!("invalid upload status '{status}'");
        }
        let now = unix_now();
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let n = guard.execute(
            "UPDATE mod_uploads SET status = ?2, note = ?3, decided_at = ?4 WHERE id = ?1",
            params![id, status, note, now],
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

    /// The most recent audit entries, newest first, capped at `limit`. Blocking.
    pub fn list_audit(&self, limit: i64) -> Result<Vec<AuditRow>> {
        let guard = self.conn.lock().expect("accounts mutex poisoned");
        let mut stmt = guard.prepare(
            "SELECT id, actor_uid, actor_login, action, target, detail, created_at
             FROM audit_log ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], |r| {
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
    fn github_sign_in_persists_user_and_resolves_session() {
        let a = Accounts::open_in_memory().unwrap();
        let sid = a.sign_in_github(42, "octocat", false).unwrap();
        let id = a.session_identity(&sid).unwrap().expect("session resolves");
        assert_eq!(id.uid, 42);
        assert_eq!(id.login, "octocat");
        assert_eq!(id.role, Role::Member);

        // a second sign-in reuses the row (no duplicate), and the allowlist can
        // promote the same uid to admin
        let sid2 = a.sign_in_github(42, "octocat-renamed", true).unwrap();
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
        let sid = a.sign_in_github(7, "x", true).unwrap();
        a.delete_session(&sid).unwrap();
        assert!(a.session_identity(&sid).unwrap().is_none());
    }

    #[test]
    fn ui_promotion_sticks_but_allowlist_stays_authoritative() {
        let a = Accounts::open_in_memory().unwrap();
        // a non-allowlisted member is promoted via the UI, then signs in again
        a.sign_in_github(5, "m", false).unwrap();
        a.set_role(5, "admin").unwrap();
        let sid = a.sign_in_github(5, "m", false).unwrap();
        assert_eq!(a.session_identity(&sid).unwrap().unwrap().role, Role::Admin);

        // an allowlisted user re-promotes to admin on login even after a demote
        a.sign_in_github(9, "op", true).unwrap();
        a.set_role(9, "member").unwrap();
        let sid2 = a.sign_in_github(9, "op", true).unwrap();
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
    fn audit_log_records_and_lists_newest_first() {
        let a = Accounts::open_in_memory().unwrap();
        a.record_audit(42, "octocat", "role.set", Some("7"), Some("admin"))
            .unwrap();
        a.record_audit(42, "octocat", "upload.approve", Some("deadbeef"), None)
            .unwrap();
        let rows = a.list_audit(10).unwrap();
        assert_eq!(rows.len(), 2);
        // newest first
        assert_eq!(rows[0].action, "upload.approve");
        assert_eq!(rows[0].target.as_deref(), Some("deadbeef"));
        assert_eq!(rows[0].detail, None);
        assert_eq!(rows[1].action, "role.set");
        assert_eq!(rows[1].actor_login, "octocat");
        assert_eq!(rows[1].detail.as_deref(), Some("admin"));
    }
}
