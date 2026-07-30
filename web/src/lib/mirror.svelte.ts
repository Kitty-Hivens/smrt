// What the mirror says changed, so a view stops asking.
//
// Every view used to answer "is this still current?" by refetching -- on a
// button, and nowhere else, which meant the answer was as stale as the last time
// somebody thought to press it. The mirror now says what moved (`/v1/events`),
// and a view listens for the one kind it renders.
//
// A change is a counter, not the data. A view watches the counter it cares about
// and refetches, and that refetch is a conditional GET the mirror usually
// answers 304 -- so listening costs a header exchange when nothing really
// changed for that view, and the view is never wrong for longer than it takes a
// message to arrive.

/// One counter per kind of change. Reading one inside `$effect` subscribes that
/// effect to it; nothing else about the event is exposed, because "something
/// you render moved" is the whole of what a view needs to act.
let registry = $state(0);
let packs = $state(0);
let moderation = $state(0);

let source: EventSource | null = null;
/// Whether this connection has ever been established. A reconnect is not the
/// same as a first connect: while the stream was down the mirror could have
/// moved without anyone hearing, so everything listening is treated as stale.
let everOpened = false;

function bumpAll() {
  registry++;
  packs++;
  moderation++;
}

export const mirror = {
  get registry() {
    return registry;
  },
  get packs() {
    return packs;
  },
  get moderation() {
    return moderation;
  },

  /// Start listening. Only for a signed-in caller -- the stream needs a session,
  /// and a guest would otherwise sit in a reconnect loop against a 401. Calling
  /// it twice is a no-op, so it is safe to call from an effect that re-runs.
  connect() {
    if (source) return;
    const src = new EventSource('/v1/events');
    source = src;
    src.addEventListener('open', () => {
      if (everOpened) bumpAll();
      everOpened = true;
    });
    src.addEventListener('registry', () => registry++);
    src.addEventListener('pack', () => packs++);
    src.addEventListener('moderation', () => moderation++);
    // The browser reconnects on its own; the `open` above is what turns that
    // reconnection into a refresh. Nothing to log -- a dropped stream is a
    // normal event on a laptop lid.
  },

  /// Stop listening -- signing out, or leaving the page.
  disconnect() {
    source?.close();
    source = null;
    everOpened = false;
  },
};
