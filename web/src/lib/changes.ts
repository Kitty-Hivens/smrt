// Reading a list of changes: how many of each, and what to call the commit
// that records them.
//
// The rows themselves come from the mirror (`ConfigChange`), so nothing here
// decides what changed -- only how a person sees it summed up, and what the
// message box can offer before anyone types. Pure functions, checked in
// `web/scripts/panel-checks.mjs`.

import type { ConfigChange } from './types';

export interface Tally {
  add: number;
  remove: number;
  change: number;
}

export function tally(rows: ConfigChange[]): Tally {
  return {
    add: rows.filter((r) => r.op === 'add').length,
    remove: rows.filter((r) => r.op === 'remove').length,
    change: rows.filter((r) => r.op === 'change').length,
  };
}

/// What a suggested commit message should say. Structure rather than a
/// sentence, because the sentence is localised and this is not: an operator
/// working in Russian gets a Russian suggestion from the same shape.
export interface Suggestion {
  /// `add` / `remove` / `update` name the things in `what`; `mixed` means the
  /// counts are the story and `what` is empty.
  kind: 'add' | 'remove' | 'update' | 'mixed';
  what: string[];
  counts: Tally;
}

/// How many names a suggestion spells out before it starts counting instead.
/// Three fits a line; a list of nine filenames is not a subject line.
const NAMED = 3;

/// A first line for the commit box, from what is about to be recorded.
///
/// It is a starting point, not a verdict -- the box stays editable and the
/// mirror still refuses an empty message. The point is that a message written
/// from memory (which is what an empty box asks for) tends to omit exactly the
/// change nobody remembers making.
export function suggest(rows: ConfigChange[]): Suggestion | null {
  if (!rows.length) return null;
  const counts = tally(rows);
  const only = (op: ConfigChange['op']) =>
    rows.every((r) => r.op === op) ? rows.map((r) => r.label) : null;

  const added = only('add');
  if (added && added.length <= NAMED) return { kind: 'add', what: added, counts };
  const removed = only('remove');
  if (removed && removed.length <= NAMED) return { kind: 'remove', what: removed, counts };
  const changed = only('change');
  if (changed && changed.length <= NAMED) {
    // A changed row is named by its label; the same file twice (a re-pin and a
    // toggle) is one name, not two.
    return { kind: 'update', what: [...new Set(changed)], counts };
  }
  return { kind: 'mixed', what: [], counts };
}
