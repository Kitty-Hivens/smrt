//! What changed on the mirror, as it changes.
//!
//! Every panel view answers "is this still current?" by asking again: the
//! registry browser refetches its listing, the moderation queue refetches its
//! queue, the catalog refetches the catalog. Asking cheaply (a conditional GET
//! that costs a `304`) is better than asking expensively, but the cheapest
//! question is the one nobody has to ask -- and polling has the worse problem
//! that the answer is stale for as long as the interval, so a harvest that
//! finished a second after the last poll is invisible for the rest of it.
//!
//! So the mirror says so. One process-wide channel carries the handful of things
//! that make a view wrong -- the mod index moved, a pack published, the
//! moderation queue changed -- and a subscriber refetches the one view that
//! cares. The event is a nudge, not the data: it says what moved and enough to
//! tell whether you care, and the refetch that follows is the same read as
//! before, usually answered `304`.
//!
//! In-process like the per-pack rooms ([`crate::authoring::PackStream`]) and the
//! job registry. A mirror is one process; an event that outlived it would be
//! describing a world that no longer exists.

use crate::accounts::Role;
use serde::Serialize;
use tokio::sync::broadcast;
use ts_rs::TS;

/// How far behind a subscriber may fall before it is dropped. Falling behind is
/// not a loss: the events say "something moved", and a reconnecting client
/// refetches what it is showing anyway.
const BACKLOG: usize = 64;

/// Something that happened to the mirror as a whole.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MirrorEvent {
    /// The mod index moved: a harvest ran, a jar was given an identity, two
    /// mods were merged, a relation was declared. `what` names which, so a view
    /// showing one mod can ignore a harvest it does not care about.
    Registry { what: String },
    /// A pack's public face changed -- a build published, a pack created,
    /// deleted, or moved between draft and published.
    Pack { pack_id: String, what: String },
    /// The upload queue moved. Operator-only: what is waiting to be moderated
    /// is not a member's business, and the count alone would still say how busy
    /// the queue is.
    Moderation { what: String },
}

impl MirrorEvent {
    /// Who this event is for. Everything the panel shows a member is theirs to
    /// hear about; the moderation queue is the operator's.
    fn reaches(&self, role: Role) -> bool {
        match self {
            MirrorEvent::Moderation { .. } => role >= Role::Admin,
            _ => true,
        }
    }

    /// The SSE event name, so a client subscribes to the kinds it renders
    /// rather than parsing every event to discard most of them.
    pub fn name(&self) -> &'static str {
        match self {
            MirrorEvent::Registry { .. } => "registry",
            MirrorEvent::Pack { .. } => "pack",
            MirrorEvent::Moderation { .. } => "moderation",
        }
    }
}

/// The mirror's change channel. One for the process, held in `AppState`.
pub struct MirrorEvents {
    tx: broadcast::Sender<MirrorEvent>,
}

impl Default for MirrorEvents {
    fn default() -> Self {
        Self {
            tx: broadcast::channel(BACKLOG).0,
        }
    }
}

impl MirrorEvents {
    /// Announce a change. Silent when nobody is listening -- a publish with no
    /// audience is the ordinary case, not an error.
    pub fn publish(&self, event: MirrorEvent) {
        let _ = self.tx.send(event);
    }

    /// Shorthands for the three kinds, so a call site says what happened rather
    /// than assembling a struct to say it.
    pub fn registry(&self, what: &str) {
        self.publish(MirrorEvent::Registry {
            what: what.to_string(),
        });
    }

    pub fn pack(&self, pack_id: &str, what: &str) {
        self.publish(MirrorEvent::Pack {
            pack_id: pack_id.to_string(),
            what: what.to_string(),
        });
    }

    pub fn moderation(&self, what: &str) {
        self.publish(MirrorEvent::Moderation {
            what: what.to_string(),
        });
    }

    /// Listen, from now on. Events published before this call are gone -- a
    /// subscriber starts by reading the world as it is, and the stream tells it
    /// what happens after that.
    pub fn subscribe(&self, role: Role) -> Subscription {
        Subscription {
            events: self.tx.subscribe(),
            role,
        }
    }
}

/// One listener, filtered to what its role may hear.
pub struct Subscription {
    events: broadcast::Receiver<MirrorEvent>,
    role: Role,
}

impl Subscription {
    /// The next event this listener may hear, skipping those it may not and
    /// those it fell too far behind to receive. `None` when the mirror is
    /// shutting down.
    pub async fn next(&mut self) -> Option<MirrorEvent> {
        loop {
            match self.events.recv().await {
                Ok(event) if event.reaches(self.role) => return Some(event),
                Ok(_) => continue,
                // too slow: the missed events all meant "refetch", and the next
                // one it does receive says the same thing
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_change_reaches_whoever_is_listening() {
        let bus = MirrorEvents::default();
        let mut member = bus.subscribe(Role::Member);
        bus.registry("harvest");
        match member.next().await.unwrap() {
            MirrorEvent::Registry { what } => assert_eq!(what, "harvest"),
            other => panic!("expected a registry change, got {other:?}"),
        }
    }

    // The queue is the operator's view. A member holding a stream open must not
    // learn from it that something is waiting to be moderated.
    #[tokio::test]
    async fn the_moderation_queue_is_not_a_members_business() {
        let bus = MirrorEvents::default();
        let mut member = bus.subscribe(Role::Member);
        let mut operator = bus.subscribe(Role::Admin);

        bus.moderation("queued");
        bus.pack("Industrial", "published");

        // the member's stream skips straight past the moderation event
        match member.next().await.unwrap() {
            MirrorEvent::Pack { pack_id, .. } => assert_eq!(pack_id, "Industrial"),
            other => panic!("a member must not hear {other:?}"),
        }
        match operator.next().await.unwrap() {
            MirrorEvent::Moderation { what } => assert_eq!(what, "queued"),
            other => panic!("expected the queue, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_are_named_so_a_client_can_pick() {
        let bus = MirrorEvents::default();
        let mut sub = bus.subscribe(Role::Admin);
        bus.registry("merge");
        assert_eq!(sub.next().await.unwrap().name(), "registry");
    }

    // Nobody listening is the ordinary case: a harvest at 4am publishes into an
    // empty room and must not care.
    #[tokio::test]
    async fn publishing_to_nobody_is_not_an_error() {
        let bus = MirrorEvents::default();
        bus.registry("harvest");
    }
}
