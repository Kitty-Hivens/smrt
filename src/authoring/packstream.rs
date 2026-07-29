//! Who is in a pack, and what has happened to it, while it is being edited.
//!
//! #52 taught the mirror to refuse a save that would overwrite someone else's:
//! the config carries a revision, a stale one comes back 409, and the editor
//! asks whose version wins. That stops the loss and says nothing until the
//! damage is already done -- you find out someone else is here by colliding with
//! them.
//!
//! This is the other half. A pack has a broadcast channel; opening the editor
//! subscribes to it and announces you; saving publishes the new revision. So an
//! editor learns that a pack moved the moment it moves, and learns that someone
//! else is in it before either of them types.
//!
//! In-process, deliberately. The mirror is one process (jobs live in memory
//! too), and a presence list that outlived the process would be a list of ghosts.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use ts_rs::TS;

/// How many events a slow subscriber may fall behind before it is dropped. A
/// dropped subscriber is not a lost edit -- the editor re-reads the config on
/// reconnect, and the revision tells it whether anything moved.
const BACKLOG: usize = 32;

/// Something that happened to a pack, as its editors see it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackEvent {
    /// The stored config changed, and who changed it. The revision is the same
    /// one `If-Match` compares, so an editor can tell "my own save came back"
    /// from "someone else saved" without asking the server anything.
    Saved { rev: String, by: String },
    /// Who is in this pack now. Sent on every join and leave rather than as a
    /// delta: the list is short, and a list is impossible to desynchronise.
    Present { editors: Vec<String> },
    /// Someone's edit, as the merge layer's own update (#115), and who made it.
    /// Base64 because this room is server-sent events, which is a text
    /// protocol -- the alternative is a second transport for one field.
    Doc { by: String, update: String },
    /// A checkpoint was declared (#122). Everyone in the pack learns that the
    /// history moved, so an editor's "changes since the last commit" count
    /// drops to zero on its own rather than staying stale until a reload.
    Committed {
        id: String,
        by: String,
        message: String,
    },
}

#[derive(Default)]
struct PackRoom {
    tx: Option<broadcast::Sender<PackEvent>>,
    /// Logins currently subscribed, with a count each: one person may have the
    /// pack open in two tabs, and leaving one of them does not make them absent.
    editors: HashMap<String, usize>,
}

/// Every pack anyone is watching. A room appears when someone joins and is left
/// behind when they all go -- an empty room is a few bytes, and reusing it keeps
/// a rejoin from racing a cleanup.
#[derive(Default)]
pub struct PackStream {
    rooms: Mutex<HashMap<String, PackRoom>>,
}

/// A subscription that announces the leave when it is dropped, so a closed tab,
/// a lost connection and a navigation all count as leaving without anything
/// having to notice which of them happened.
pub struct Presence {
    stream: Arc<PackStream>,
    pack_id: String,
    login: String,
    pub events: broadcast::Receiver<PackEvent>,
}

impl PackStream {
    /// Join a pack's room as `login`, receiving its events from now on. The
    /// current roster arrives immediately, so a joiner does not wait for someone
    /// else to move before it knows who is here.
    pub fn join(self: &Arc<Self>, pack_id: &str, login: &str) -> Presence {
        let (events, roster) = {
            let mut rooms = self.rooms.lock().unwrap();
            let room = rooms.entry(pack_id.to_string()).or_default();
            let tx = room
                .tx
                .get_or_insert_with(|| broadcast::channel(BACKLOG).0)
                .clone();
            *room.editors.entry(login.to_string()).or_insert(0) += 1;
            (tx.subscribe(), room_roster(room))
        };
        // announced after the subscription exists, so the joiner sees itself in
        // the roster it is told about
        self.publish(pack_id, PackEvent::Present { editors: roster });
        Presence {
            stream: self.clone(),
            pack_id: pack_id.to_string(),
            login: login.to_string(),
            events,
        }
    }

    /// Send an event to everyone in a pack. Silent when nobody is there: a save
    /// with no audience is not an error.
    pub fn publish(&self, pack_id: &str, event: PackEvent) {
        let tx = {
            let rooms = self.rooms.lock().unwrap();
            rooms.get(pack_id).and_then(|r| r.tx.clone())
        };
        if let Some(tx) = tx {
            let _ = tx.send(event);
        }
    }

    /// Who the mirror believes is in this pack.
    pub fn editors(&self, pack_id: &str) -> Vec<String> {
        let rooms = self.rooms.lock().unwrap();
        rooms.get(pack_id).map(room_roster).unwrap_or_default()
    }

    fn leave(&self, pack_id: &str, login: &str) -> Option<Vec<String>> {
        let mut rooms = self.rooms.lock().unwrap();
        let room = rooms.get_mut(pack_id)?;
        match room.editors.get_mut(login) {
            Some(n) if *n > 1 => *n -= 1,
            Some(_) => {
                room.editors.remove(login);
            }
            None => return None,
        }
        Some(room_roster(room))
    }
}

fn room_roster(room: &PackRoom) -> Vec<String> {
    let mut names: Vec<String> = room.editors.keys().cloned().collect();
    names.sort();
    names
}

impl Drop for Presence {
    fn drop(&mut self) {
        if let Some(editors) = self.stream.leave(&self.pack_id, &self.login) {
            self.stream
                .publish(&self.pack_id, PackEvent::Present { editors });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream() -> Arc<PackStream> {
        Arc::new(PackStream::default())
    }

    #[tokio::test]
    async fn joining_announces_the_roster_and_leaving_retracts_it() {
        let s = stream();
        let mut a = s.join("Industrial", "ada");
        // the joiner sees itself: a roster it is not in would be a list of
        // other people, which is not what "who is here" means
        match a.events.try_recv().unwrap() {
            PackEvent::Present { editors } => assert_eq!(editors, vec!["ada".to_string()]),
            other => panic!("expected a roster, got {other:?}"),
        }

        let b = s.join("Industrial", "bo");
        match a.events.try_recv().unwrap() {
            PackEvent::Present { editors } => {
                assert_eq!(editors, vec!["ada".to_string(), "bo".to_string()])
            }
            other => panic!("expected a roster, got {other:?}"),
        }

        drop(b);
        match a.events.try_recv().unwrap() {
            PackEvent::Present { editors } => assert_eq!(editors, vec!["ada".to_string()]),
            other => panic!("expected a roster, got {other:?}"),
        }
        assert_eq!(s.editors("Industrial"), vec!["ada".to_string()]);
    }

    // One person, two tabs. Closing one must not report them as gone, or the
    // roster becomes a lie the moment anyone works the way people actually do.
    #[tokio::test]
    async fn a_second_tab_does_not_double_a_person_or_remove_them_early() {
        let s = stream();
        let first = s.join("Industrial", "ada");
        let second = s.join("Industrial", "ada");
        assert_eq!(s.editors("Industrial"), vec!["ada".to_string()]);

        drop(second);
        assert_eq!(
            s.editors("Industrial"),
            vec!["ada".to_string()],
            "still here through the other tab"
        );
        drop(first);
        assert!(s.editors("Industrial").is_empty());
    }

    #[tokio::test]
    async fn a_save_reaches_everyone_in_the_pack_and_nobody_outside_it() {
        let s = stream();
        let mut here = s.join("Industrial", "ada");
        let mut elsewhere = s.join("Create", "bo");
        let _ = here.events.try_recv(); // the joins
        let _ = elsewhere.events.try_recv();

        s.publish(
            "Industrial",
            PackEvent::Saved {
                rev: "abc123".into(),
                by: "bo".into(),
            },
        );
        match here.events.try_recv().unwrap() {
            PackEvent::Saved { rev, by } => {
                assert_eq!(rev, "abc123");
                assert_eq!(by, "bo");
            }
            other => panic!("expected a save, got {other:?}"),
        }
        assert!(
            elsewhere.events.try_recv().is_err(),
            "another pack's editors hear nothing"
        );
    }
}
