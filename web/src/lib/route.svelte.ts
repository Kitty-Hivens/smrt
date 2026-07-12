// Which section the shell is showing. A module-level $state shared by the rail
// (which highlights + sets it) and the content area (which renders by it).
// Persisted so a refresh keeps you where you were.

export type Section = 'browse' | 'overview' | 'packs' | 'servers' | 'mods' | 'users';
export const SECTIONS: Section[] = ['browse', 'overview', 'packs', 'servers', 'mods', 'users'];
// Sections a guest or member may open; everything else is operator-only.
export const PUBLIC_SECTIONS: Section[] = ['browse'];
export function visibleSections(isAdmin: boolean): Section[] {
  return isAdmin ? SECTIONS : PUBLIC_SECTIONS;
}

const STORAGE_KEY = 'smrt.section';

function initial(): Section {
  try {
    const s = localStorage.getItem(STORAGE_KEY);
    // the old sha1 'cache' tab was replaced by mod management
    if (s === 'cache') return 'mods';
    if (s && SECTIONS.includes(s as Section)) return s as Section;
  } catch {
    // blocked storage -- default below
  }
  return 'browse';
}

let section = $state<Section>(initial());

export const route = {
  get section(): Section {
    return section;
  },
  go(s: Section) {
    section = s;
    try {
      localStorage.setItem(STORAGE_KEY, s);
    } catch {
      // session-only navigation still works
    }
  },
};
