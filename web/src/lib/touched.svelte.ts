// Who just changed what, and where (#115 follow-on).
//
// With edits merging live, someone else's work appears on screen without a
// word about whose it is or that it happened at all. Presence says who is in
// the pack; this says what they touched.
//
// The unit is a field over a short window, not a person. A paragraph five people
// are writing has no owner, and a marker naming one of them would be a lie --
// so a field carries the set of whoever touched it recently, and the render
// decides whether that reads as a name, two names, or a count. A scalar has
// exactly one author by construction (the last write wins), so the ambiguity is
// only ever real for prose and lists.
//
// The window is short on purpose. Five people over an hour is not five people
// at once; holding only the last few seconds makes "5 people" appear exactly
// when five of them are actually there.

/// How long a change stays marked. Long enough to notice a colleague working,
/// short enough that the marker means "now".
const WINDOW_MS = 6000;

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/// Every path whose value differs between two states of the config.
///
/// Arrays are compared by position and reported at the row: a mod row is the
/// thing a person edits, and saying "mods.3" is an address someone can act on
/// where "mods" is not. A row appearing or disappearing is reported as the row,
/// not as every field inside it.
export function changedPaths(before: Json, after: Json, prefix = ''): string[] {
  if (before === after) return [];
  const bothObjects =
    before !== null &&
    after !== null &&
    typeof before === 'object' &&
    typeof after === 'object' &&
    Array.isArray(before) === Array.isArray(after);
  if (!bothObjects) return prefix ? [prefix] : [];

  if (Array.isArray(before) && Array.isArray(after)) {
    const out: string[] = [];
    const shared = Math.min(before.length, after.length);
    for (let i = 0; i < shared; i++) {
      if (JSON.stringify(before[i]) !== JSON.stringify(after[i])) out.push(`${prefix}.${i}`);
    }
    for (let i = shared; i < Math.max(before.length, after.length); i++) {
      out.push(`${prefix}.${i}`);
    }
    return out;
  }

  const a = before as { [key: string]: Json };
  const b = after as { [key: string]: Json };
  const out: string[] = [];
  for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) {
    const at = prefix ? `${prefix}.${key}` : key;
    out.push(...changedPaths(a[key] ?? null, b[key] ?? null, at));
  }
  return out;
}

export type Touch = { paths: string[]; who: string; at: number };

/// What has been touched lately, and by whom.
///
/// A plain object keyed by path rather than one entry per event: the question a
/// marker asks is "who has been in this field", and a list of events would have
/// the render deduplicate on every frame.
export function createTouches(now: () => number = () => Date.now()) {
  let entries = $state<Record<string, { who: string[]; at: number }>>({});

  const fresh = (at: number) => now() - at < WINDOW_MS;

  return {
    /// Record someone's change. Re-touching a field extends its window and adds
    /// the person, so two people in one paragraph read as two rather than as
    /// whoever moved last.
    record(paths: string[], who: string) {
      if (!who || paths.length === 0) return;
      const next = { ...entries };
      for (const path of paths) {
        const current = fresh(next[path]?.at ?? 0) ? next[path] : undefined;
        const people = current?.who ?? [];
        next[path] = {
          who: people.includes(who) ? people : [...people, who],
          at: now(),
        };
      }
      entries = next;
    },

    /// Who touched this path lately, newest window first. Empty once it decays.
    who(path: string): string[] {
      const e = entries[path];
      return e && fresh(e.at) ? e.who : [];
    },

    /// Every path still inside its window, so a summary can be rendered without
    /// asking about paths nobody touched.
    get live(): { path: string; who: string[] }[] {
      return Object.entries(entries)
        .filter(([, e]) => fresh(e.at))
        .sort((a, b) => b[1].at - a[1].at)
        .map(([path, e]) => ({ path, who: e.who }));
    },

    /// Drop what has decayed. Called on a timer by the view: without it the map
    /// would grow for the lifetime of the editor, and `live` would filter an
    /// ever-longer list on every render.
    sweep() {
      const kept = Object.entries(entries).filter(([, e]) => fresh(e.at));
      if (kept.length !== Object.keys(entries).length) {
        entries = Object.fromEntries(kept);
      }
    },
  };
}
