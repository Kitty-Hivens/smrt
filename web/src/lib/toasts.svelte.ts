// Transient notices, rendered in a fixed stack by Toaster.svelte.
//
// These used to be inline blocks at the top of a view: pressing anything that
// could fail inserted a banner and shoved the whole form down, and the operator
// lost their place mid-edit. A notice never displaces content now.
//
// A sticky notice stays until dismissed or replaced -- it reports a state that
// is still true (a save the server refused), not an event that happened.

export type ToastKind = 'error' | 'ok' | 'info';

export type Toast = {
  id: number;
  kind: ToastKind;
  text: string;
  /// Machine detail (a server message), shown small and monospaced under the text.
  detail?: string;
  action?: { label: string; run: () => void };
  sticky?: boolean;
};

const AUTO_DISMISS_MS = 6000;

let seq = 0;
let items = $state<Toast[]>([]);
const timers = new Map<number, ReturnType<typeof setTimeout>>();

function clearTimer(id: number) {
  const t = timers.get(id);
  if (t !== undefined) {
    clearTimeout(t);
    timers.delete(id);
  }
}

export const toasts = {
  get list(): Toast[] {
    return items;
  },
  /// Show a notice; returns its id so a caller holding a state (a failing save)
  /// can replace or dismiss its own notice instead of stacking duplicates.
  push(t: Omit<Toast, 'id'>): number {
    const id = ++seq;
    items = [...items, { ...t, id }];
    if (!t.sticky) timers.set(id, setTimeout(() => toasts.dismiss(id), AUTO_DISMISS_MS));
    return id;
  },
  /// Replace a notice in place, keeping its slot in the stack so a retried
  /// failure does not make the pile grow.
  replace(id: number | null, t: Omit<Toast, 'id'>): number {
    if (id === null || !items.some((x) => x.id === id)) return toasts.push(t);
    clearTimer(id);
    items = items.map((x) => (x.id === id ? { ...t, id } : x));
    if (!t.sticky) timers.set(id, setTimeout(() => toasts.dismiss(id), AUTO_DISMISS_MS));
    return id;
  },
  dismiss(id: number | null) {
    if (id === null) return;
    clearTimer(id);
    items = items.filter((x) => x.id !== id);
  },
};
