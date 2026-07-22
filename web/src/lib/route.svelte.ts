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
// The operator's official tabs. `mypacks` is not among them because it is a
// personal surface, not an operator one -- but every account has it, admins
// included (their own community packs under `u/<uid>/`, distinct from the
// official packs they author via `packs`), so `visibleSections` appends it for
// operators too. KNOWN_SECTIONS is the superset used to validate a stored tab.
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
// The registry browser (mods) and the graph are read-only for a member -- the
// views gate their own authoring, and the data is already public per-mod on the
// mod page -- so a member building a community pack gets the same read of what the
// mirror indexes, and the same "does this hold together" view, an operator has.
export const MEMBER_SECTIONS: Section[] = ['browse', 'mods', 'graph', 'mypacks', 'profile'];
const KNOWN_SECTIONS: Section[] = [...SECTIONS, 'mypacks'];
export function visibleSections(me: { role: string } | null): Section[] {
  if (!me) return GUEST_SECTIONS;
  if (isOperator(me.role)) return [...SECTIONS, 'mypacks'];
  return MEMBER_SECTIONS;
}

const STORAGE_KEY = 'smrt.section';

// The panel's state lives in the URL. It did not before: navigation was a
// variable plus a localStorage key, so the browser had no idea anything had
// happened -- back and forward (and the mouse buttons wired to them) did
// nothing or left the app entirely, a reload lost an open mod page, and there
// was no way to send anyone a link to what you were looking at.
//
// `/` restores your last section, `/<section>` selects one, `/mod/<ref>` opens
// a mod over it. The server serves the app shell for any path it does not
// claim, so these survive a reload.

function sectionFromPath(path: string): Section | null {
  const seg = path.replace(/^\/+|\/+$/g, '').split('/')[0];
  // the old sha1 'cache' tab was replaced by mod management
  if (seg === 'cache') return 'mods';
  return KNOWN_SECTIONS.includes(seg as Section) ? (seg as Section) : null;
}

function storedSection(): Section {
  try {
    const s = localStorage.getItem(STORAGE_KEY);
    if (s === 'cache') return 'mods';
    if (s && KNOWN_SECTIONS.includes(s as Section)) return s as Section;
  } catch {
    // blocked storage -- default below
  }
  return 'browse';
}

/// A mod ref out of `/mod/<ref>`: a numeric id, or `sha1:<hash>` for a jar the
/// pack knows by hash rather than by registry id.
function modFromPath(path: string): string | null {
  const m = path.match(/^\/mod\/(.+)$/);
  return m ? decodeURIComponent(m[1]) : null;
}

function initial(): Section {
  return sectionFromPath(location.pathname) ?? storedSection();
}

let section = $state<Section>(initial());
// A focused mod page overlays whatever section is active: set, the content area
// renders the mod page instead of the section; cleared, it returns to `section`.
// Reachable from the registry, a pack's mod list, and the graph, so it lives here
// rather than as one view's local state. Not persisted -- a refresh lands on the
// underlying section, not a deep mod link (the store has no URL to restore from).
// The value is a mod ref the API accepts: a numeric id (graph / registry) or
// `sha1:<hash>` (a pack's mod list has the jar's sha1, not the id).
let focusMod = $state<string | null>(modFromPath(location.pathname));

/// Push a URL for a state the user navigated to, so it becomes a history entry
/// they can come back from. Replacing (rather than pushing) the very first
/// entry keeps `/` from sitting behind every session as a dead step.
function pushPath(path: string, replace = false) {
  if (location.pathname === path) return;
  history[replace ? 'replaceState' : 'pushState']({}, '', path);
}

function remember(s: Section) {
  try {
    localStorage.setItem(STORAGE_KEY, s);
  } catch {
    // session-only navigation still works
  }
}

// The URL is the truth on the way back: whatever the browser restores, the
// store follows -- without pushing, or every back press would leave a new entry.
if (typeof window !== 'undefined') {
  window.addEventListener('popstate', () => {
    const mod = modFromPath(location.pathname);
    focusMod = mod;
    if (!mod) {
      const s = sectionFromPath(location.pathname);
      if (s) {
        section = s;
        remember(s);
      }
    }
  });
  // a bare `/` restores the last section without leaving an extra entry behind
  if (!sectionFromPath(location.pathname) && !modFromPath(location.pathname)) {
    pushPath(`/${section}`, true);
  }
  remember(section);
}

export const route = {
  get section(): Section {
    return section;
  },
  get mod(): string | null {
    return focusMod;
  },
  /// `replace` is for a correction rather than a navigation -- landing on a
  /// section your role cannot see should not leave a step to go back to.
  go(s: Section, replace = false) {
    focusMod = null; // picking a section leaves any open mod page
    section = s;
    remember(s);
    pushPath(`/${s}`, replace);
  },
  // Open a mod's page over the current section; `closeMod` returns to it. `ref`
  // is a numeric mod id or a `sha1:<hash>` artifact reference.
  openMod(ref: number | string) {
    focusMod = String(ref);
    pushPath(`/mod/${encodeURIComponent(String(ref))}`);
  },
  closeMod() {
    focusMod = null;
    // back rather than a fresh entry: the mod page was opened from the section
    // underneath, and closing it is the same move as pressing back
    if (modFromPath(location.pathname)) history.back();
  },
};
