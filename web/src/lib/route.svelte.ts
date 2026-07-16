// Which section the shell is showing. A module-level $state shared by the rail
// (which highlights + sets it) and the content area (which renders by it).
// Persisted so a refresh keeps you where you were.

import { isOperator } from './roles';

export type Section =
  | 'browse'
  | 'overview'
  | 'packs'
  | 'servers'
  | 'mods'
  | 'graph'
  | 'users'
  | 'moderation'
  | 'audit'
  | 'profile'
  | 'mypacks';
// The operator's tabs. `mypacks` is member-only (admins author via `packs`), so
// it is not here; KNOWN_SECTIONS is the superset used to validate a stored tab.
export const SECTIONS: Section[] = [
  'browse',
  'overview',
  'packs',
  'servers',
  'mods',
  'graph',
  'users',
  'moderation',
  'audit',
  'profile',
];
// Guest sees only the public catalog; a signed-in member also gets their own
// packs and profile; everything else is operator-only.
export const GUEST_SECTIONS: Section[] = ['browse'];
// The registry graph is read-only for a member (authoring edges is debug-gated in
// the view), and its relation data is already public per-mod on the mod page, so a
// member authoring community packs gets the same "does this hold together" view an
// operator has.
export const MEMBER_SECTIONS: Section[] = ['browse', 'graph', 'mypacks', 'profile'];
const KNOWN_SECTIONS: Section[] = [...SECTIONS, 'mypacks'];
export function visibleSections(me: { role: string } | null): Section[] {
  if (!me) return GUEST_SECTIONS;
  if (isOperator(me.role)) return SECTIONS;
  return MEMBER_SECTIONS;
}

const STORAGE_KEY = 'smrt.section';

function initial(): Section {
  try {
    const s = localStorage.getItem(STORAGE_KEY);
    // the old sha1 'cache' tab was replaced by mod management
    if (s === 'cache') return 'mods';
    if (s && KNOWN_SECTIONS.includes(s as Section)) return s as Section;
  } catch {
    // blocked storage -- default below
  }
  return 'browse';
}

let section = $state<Section>(initial());
// A focused mod page overlays whatever section is active: set, the content area
// renders the mod page instead of the section; cleared, it returns to `section`.
// Reachable from the registry, a pack's mod list, and the graph, so it lives here
// rather than as one view's local state. Not persisted -- a refresh lands on the
// underlying section, not a deep mod link (the store has no URL to restore from).
// The value is a mod ref the API accepts: a numeric id (graph / registry) or
// `sha1:<hash>` (a pack's mod list has the jar's sha1, not the id).
let focusMod = $state<string | null>(null);

export const route = {
  get section(): Section {
    return section;
  },
  get mod(): string | null {
    return focusMod;
  },
  go(s: Section) {
    focusMod = null; // picking a section leaves any open mod page
    section = s;
    try {
      localStorage.setItem(STORAGE_KEY, s);
    } catch {
      // session-only navigation still works
    }
  },
  // Open a mod's page over the current section; `closeMod` returns to it. `ref`
  // is a numeric mod id or a `sha1:<hash>` artifact reference.
  openMod(ref: number | string) {
    focusMod = String(ref);
  },
  closeMod() {
    focusMod = null;
  },
};
