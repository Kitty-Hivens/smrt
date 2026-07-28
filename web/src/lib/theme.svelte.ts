// Which half of the token file is in force.
//
// Three choices, not two: dark, light, and following the system. Following is
// the default because the panel cannot know which the operator wants, and
// guessing dark for someone whose desktop is light is the same rudeness as the
// reverse. The resolved value is always written to the root as `data-theme`, so
// the CSS never has to reason about preference -- one attribute, one answer.

import { withTransition } from './transition.svelte';

export type ThemeChoice = 'system' | 'dark' | 'light';
export type Resolved = 'dark' | 'light';

const STORAGE_KEY = 'smrt.theme';
const DARK_DEFAULT: Resolved = 'dark';

function stored(): ThemeChoice {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === 'dark' || v === 'light' || v === 'system') return v;
  } catch {
    // blocked storage: the session still switches, it just does not persist
  }
  return 'system';
}

function systemPrefers(): Resolved {
  if (typeof window === 'undefined' || !window.matchMedia) return DARK_DEFAULT;
  // Only an explicit light preference counts. A browser with no preference
  // reports "light" for `prefers-color-scheme: light` in some engines and
  // nothing in others, so the question is asked the way round that has one
  // answer: is light asked for.
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : DARK_DEFAULT;
}

let choice = $state<ThemeChoice>(stored());
let resolved = $state<Resolved>(choice === 'system' ? systemPrefers() : choice);

function apply(r: Resolved) {
  document.documentElement.setAttribute('data-theme', r);
}

if (typeof window !== 'undefined') {
  apply(resolved);
  // following the system means following it as it changes, not as it was at load
  window
    .matchMedia?.('(prefers-color-scheme: light)')
    .addEventListener?.('change', () => {
      if (choice !== 'system') return;
      resolved = systemPrefers();
      apply(resolved);
    });
}

export const theme = {
  get choice(): ThemeChoice {
    return choice;
  },
  /// What is actually painted right now: `system` resolved against the desktop.
  get resolved(): Resolved {
    return resolved;
  },
  set(next: ThemeChoice) {
    choice = next;
    resolved = next === 'system' ? systemPrefers() : next;
    // A substrate swap is a shorter act than a rewrite of the text: the shapes
    // stay where they are and only their colours move.
    withTransition('theme', () => apply(resolved));
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // session-only
    }
  },
};
