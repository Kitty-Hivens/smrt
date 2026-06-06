// Which section the shell is showing. A module-level $state shared by the rail
// (which highlights + sets it) and the content area (which renders by it).
// Persisted so a refresh keeps you where you were.

export type Section = 'overview' | 'packs' | 'servers' | 'cache';
export const SECTIONS: Section[] = ['overview', 'packs', 'servers', 'cache'];

const STORAGE_KEY = 'smrt.section';

function initial(): Section {
  try {
    const s = localStorage.getItem(STORAGE_KEY);
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
