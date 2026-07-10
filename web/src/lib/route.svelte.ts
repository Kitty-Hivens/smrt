// Which section the shell is showing. A module-level $state shared by the rail
// (which highlights + sets it) and the content area (which renders by it).
// Persisted so a refresh keeps you where you were.

export type Section = 'overview' | 'packs' | 'servers' | 'mods';
export const SECTIONS: Section[] = ['overview', 'packs', 'servers', 'mods'];

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
  return 'overview';
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
