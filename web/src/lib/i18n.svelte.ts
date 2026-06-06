// Tiny reactive i18n. A module-level $state holds the active locale; `t()` reads
// it, so any component calling `t(...)` in its markup re-renders on a switch.
// Hand-rolled rather than a dependency: two locales, flat keys, no plurals yet.

import { en, type Dict } from './locales/en';
import { ru } from './locales/ru';

export type Locale = 'ru' | 'en';
export const LOCALES: Locale[] = ['ru', 'en'];

const dicts: Record<Locale, Dict> = { ru, en };
const STORAGE_KEY = 'smrt.locale';

function initialLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === 'ru' || saved === 'en') return saved;
  } catch {
    // private mode / blocked storage -- fall through to default
  }
  return 'ru';
}

const startLocale = initialLocale();
let current = $state<Locale>(startLocale);
if (typeof document !== 'undefined') document.documentElement.lang = startLocale;

export const i18n = {
  get locale(): Locale {
    return current;
  },
  set(loc: Locale) {
    current = loc;
    try {
      localStorage.setItem(STORAGE_KEY, loc);
    } catch {
      // ignore -- in-memory locale still works for the session
    }
    if (typeof document !== 'undefined') document.documentElement.lang = loc;
  },
  toggle() {
    this.set(current === 'ru' ? 'en' : 'ru');
  },
};

export type MsgKey = keyof Dict;

export function t(key: MsgKey, params?: Record<string, string | number>): string {
  let s: string = dicts[current][key] ?? en[key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      s = s.replaceAll(`{${k}}`, String(v));
    }
  }
  return s;
}
