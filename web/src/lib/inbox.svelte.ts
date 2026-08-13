// What the mirror has to tell this account: reports answered, proposals
// decided, threads opened on packs it keeps.
//
// One store rather than a fetch per view, because two surfaces read the same
// answer -- the rail shows how many are unread, the profile shows what they
// are -- and a badge that disagreed with the list under it would be worse than
// no badge. It is refreshed on the same event the rest of the panel listens to:
// a discussion moving is a `pack` change, so nothing here polls.

import { api } from './api';
import type { Notification } from './types';

let unread = $state(0);
let rows = $state<Notification[]>([]);
let loaded = $state(false);

export const inbox = {
  /// How many are unread in total -- not how many rows are loaded.
  get unread() {
    return unread;
  },
  get rows() {
    return rows;
  },
  /// Whether the list has ever been read, so a view can tell "nothing yet" from
  /// "not asked yet".
  get loaded() {
    return loaded;
  },

  /// Re-read the list. A guest has none, and a failure leaves what was there:
  /// an inbox that empties itself because one request failed would read as
  /// "somebody dealt with it".
  async refresh(): Promise<void> {
    try {
      const listing = await api.notifications(false, 50);
      unread = listing.unread;
      rows = listing.rows;
      loaded = true;
    } catch {
      // keep whatever was already shown
    }
  },

  /// Mark one read, or all of them, and follow the mirror's answer rather than
  /// guessing at it locally.
  async markRead(id?: number): Promise<void> {
    await api.markNotificationsRead(id);
    await this.refresh();
  },

  /// Forget everything on sign-out, so the next account does not inherit a
  /// count that was never theirs.
  clear(): void {
    unread = 0;
    rows = [];
    loaded = false;
  },
};
