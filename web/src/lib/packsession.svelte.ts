// One editor's seat in a pack (#115).
//
// The mirror holds the document and merges what everyone sends it; this is the
// connection to it: catch up on open, send what this editor changes, take what
// the others change, and say when the two are out of step.
//
// Sending is a POST and receiving rides the pack's existing event room, which
// is one-way. Two doors rather than a socket, because that is what already
// exists and passes through the deployment untouched -- a WebSocket would be a
// new protocol to terminate for one field.

import { api } from './api';
import { readConfig, writeConfig } from './packdoc';
import type { PackConfig } from './types';
import * as Y from 'yjs';

/// Origins on a document update, so the update handler can tell what to send.
/// A remote update must not be echoed back where it came from, and the seed
/// must not be sent at all.
const LOCAL = 'local';
const REMOTE = 'remote';

export type PackSession = {
  /// Put what is on screen into the document. A no-op when nothing differs, so
  /// it is safe to call on every keystroke and on every remote change.
  push: (cfg: PackConfig) => void;
  /// Take an update from the pack's room, and say who sent it.
  receive: (base64: string, by: string) => void;
  /// The document as a config, with the server's fields from the loaded one.
  read: () => PackConfig;
  close: () => void;
};

function decode(base64: string): Uint8Array {
  const raw = atob(base64);
  const out = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
  return out;
}

function encode(bytes: Uint8Array): string {
  let raw = '';
  for (const b of bytes) raw += String.fromCharCode(b);
  return btoa(raw);
}

/// Join a pack's document. `base` is the config this editor loaded, which
/// supplies the server-owned fields the document deliberately does not carry.
///
/// `onRemote` fires after someone else's change is merged, with the merged
/// config and whose change it was: the editor adopts it rather than re-reading,
/// which is the difference between seeing a colleague type and being
/// interrupted by a reload, and it can say whose work just appeared.
export async function openPackSession(
  packId: string,
  base: PackConfig,
  onRemote: (merged: PackConfig, by: string) => void,
): Promise<PackSession> {
  const doc = new Y.Doc();

  // Catch up by applying the mirror's state to an EMPTY document. Seeding from
  // the config first and then applying this would author a second value for
  // every key, concurrently with the mirror's, and one whole `mods` array would
  // replace the other along with everything in it.
  Y.applyUpdate(doc, await api.packDocState(packId), REMOTE);

  let closed = false;
  const send = (update: Uint8Array, origin: unknown) => {
    if (closed || origin !== LOCAL) return;
    // Fire and forget: the document keeps the change either way, and the next
    // edit carries everything the mirror has not seen. A failed send is a
    // dropped frame, not a lost edit.
    void api.sendPackDoc(packId, update).catch(() => {});
  };
  doc.on('update', send);

  return {
    push(cfg) {
      if (!closed) writeConfig(doc, cfg, LOCAL);
    },
    receive(base64, by) {
      if (closed) return;
      Y.applyUpdate(doc, decode(base64), REMOTE);
      onRemote(readConfig(doc, base), by);
    },
    read: () => readConfig(doc, base),
    close() {
      closed = true;
      doc.off('update', send);
      doc.destroy();
    },
  };
}

export const __test = { decode, encode };
