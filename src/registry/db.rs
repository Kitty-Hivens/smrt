//! Embedded SQLite handle for the mod registry. rusqlite is synchronous, so the
//! connection sits behind a `std::sync::Mutex` and every caller runs its closure
//! inside `tokio::task::spawn_blocking` (same idiom as the unzip/build paths).

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

use super::migrations;

pub struct Registry {
    conn: Mutex<Connection>,
}

impl Registry {
    /// Open (creating if absent) the registry DB at `path`, set pragmas, and
    /// apply pending migrations. Synchronous; called once at startup.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path)
            .with_context(|| format!("opening registry db at {}", path.display()))?;
        Self::init(conn)
    }

    /// In-memory registry, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().context("opening in-memory registry")?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        // WAL lets the readers proceed alongside the single harvest writer; the
        // busy_timeout absorbs the brief write lock. (WAL is a no-op on :memory:.)
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .context("setting registry pragmas")?;
        migrations::apply_pending(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Run a read against the connection. Call inside `spawn_blocking`.
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.conn.lock().expect("registry mutex poisoned");
        f(&guard)
    }

    /// Run a write/transaction needing `&mut Connection`. Call inside
    /// `spawn_blocking`.
    pub fn with_conn_mut<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut guard = self.conn.lock().expect("registry mutex poisoned");
        f(&mut guard)
    }

    /// Run `f` inside a single transaction, committing on `Ok`. `f` receives a
    /// `&Connection` (a `&Transaction` deref-coerces), so the upsert helpers
    /// take part in one atomic write. Call inside `spawn_blocking`.
    pub fn with_txn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut guard = self.conn.lock().expect("registry mutex poisoned");
        let tx = guard.transaction().context("begin registry txn")?;
        let out = f(&tx)?;
        tx.commit().context("commit registry txn")?;
        Ok(out)
    }
}
