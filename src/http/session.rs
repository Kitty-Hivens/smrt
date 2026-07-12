//! Server-side panel sessions. The browser cookie carries only an opaque id;
//! the identity (GitHub uid/login + role) lives here. So no credential rides in
//! the browser, and logout revokes server-side. In-memory: a process restart
//! asks the operator to sign in again, acceptable for a single-operator panel.

use rand::RngCore;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const SESSION_TTL: Duration = Duration::from_secs(86_400);

/// The panel's authorization tiers. Only `Admin` exists today; the reader /
/// scoped-curator ladder arrives with the roles work and slots in here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
        }
    }
}

/// Who is behind a request, once authenticated. Attached to request extensions
/// by the auth middleware and read by handlers that report the current user.
#[derive(Clone, Debug)]
pub struct Identity {
    /// GitHub numeric uid, or 0 for the break-glass admin token.
    pub uid: u64,
    pub login: String,
    pub role: Role,
}

struct Entry {
    identity: Identity,
    expires: Instant,
}

#[derive(Default)]
pub struct SessionStore {
    entries: Mutex<HashMap<String, Entry>>,
}

impl SessionStore {
    /// Store an identity under a fresh opaque id, returned for the cookie.
    pub fn create(&self, identity: Identity) -> String {
        let id = random_token();
        self.entries.lock().unwrap().insert(
            id.clone(),
            Entry {
                identity,
                expires: Instant::now() + SESSION_TTL,
            },
        );
        id
    }

    /// The identity behind a session id, if it exists and has not expired. A
    /// lapsed entry is dropped on read so the map self-prunes as ids are seen.
    pub fn get(&self, id: &str) -> Option<Identity> {
        let mut m = self.entries.lock().unwrap();
        match m.get(id) {
            Some(e) if e.expires > Instant::now() => Some(e.identity.clone()),
            Some(_) => {
                m.remove(id);
                None
            }
            None => None,
        }
    }

    pub fn remove(&self, id: &str) {
        self.entries.lock().unwrap().remove(id);
    }
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
    fn create_then_get_roundtrips_identity() {
        let store = SessionStore::default();
        let id = store.create(Identity {
            uid: 42,
            login: "octocat".into(),
            role: Role::Admin,
        });
        let got = store.get(&id).expect("session should resolve");
        assert_eq!(got.uid, 42);
        assert_eq!(got.login, "octocat");
        assert_eq!(got.role, Role::Admin);
    }

    #[test]
    fn removed_session_no_longer_resolves() {
        let store = SessionStore::default();
        let id = store.create(Identity {
            uid: 1,
            login: "a".into(),
            role: Role::Admin,
        });
        store.remove(&id);
        assert!(store.get(&id).is_none());
    }

    #[test]
    fn random_tokens_are_distinct_and_sized() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64); // 32 bytes hex-encoded
    }
}
