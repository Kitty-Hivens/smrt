// Promise-based in-panel dialogs replacing window.confirm / window.prompt.
// A single <DialogHost> renders the active request; callers `await` the result.

import { t } from './i18n.svelte';

interface ConfirmReq {
  kind: 'confirm';
  title: string;
  message: string;
  danger: boolean;
  resolve: (v: boolean) => void;
}

interface PromptReq {
  kind: 'prompt';
  title: string;
  label: string;
  initial: string;
  placeholder: string;
  resolve: (v: string | null) => void;
}

type Req = ConfirmReq | PromptReq;

let active = $state<Req | null>(null);

// Settle an already-open dialog as cancelled before a new one replaces it, so
// the superseded promise never hangs (and its caller's busy flag clears).
function settlePending() {
  if (active?.kind === 'confirm') active.resolve(false);
  else if (active?.kind === 'prompt') active.resolve(null);
}

export const dialogs = {
  get active(): Req | null {
    return active;
  },

  confirm(message: string, opts: { title?: string; danger?: boolean } = {}): Promise<boolean> {
    return new Promise((resolve) => {
      settlePending();
      active = {
        kind: 'confirm',
        title: opts.title ?? t('dialog.confirmTitle'),
        message,
        danger: opts.danger ?? false,
        resolve,
      };
    });
  },

  prompt(
    label: string,
    opts: { title?: string; initial?: string; placeholder?: string } = {},
  ): Promise<string | null> {
    return new Promise((resolve) => {
      settlePending();
      active = {
        kind: 'prompt',
        title: opts.title ?? t('dialog.inputTitle'),
        label,
        initial: opts.initial ?? '',
        placeholder: opts.placeholder ?? '',
        resolve,
      };
    });
  },

  resolveConfirm(value: boolean): void {
    const a = active;
    active = null;
    if (a?.kind === 'confirm') a.resolve(value);
  },

  resolvePrompt(value: string | null): void {
    const a = active;
    active = null;
    if (a?.kind === 'prompt') a.resolve(value);
  },
};
