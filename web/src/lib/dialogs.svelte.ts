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

/// One way out of a `choose`. `danger` marks the destructive one so it reads as
/// the deliberate act it is.
export interface ChoiceOption {
  value: string;
  label: string;
  danger?: boolean;
}

/// A question with more than two answers, where confirm/cancel would have to
/// hide one of them behind an unlabelled button -- a save conflict, where
/// loading the server's version, overwriting it, and neither are three
/// different decisions.
interface ChooseReq {
  kind: 'choose';
  title: string;
  message: string;
  options: ChoiceOption[];
  resolve: (v: string | null) => void;
}

type Req = ConfirmReq | PromptReq | ChooseReq;

let active = $state<Req | null>(null);

// Settle an already-open dialog as cancelled before a new one replaces it, so
// the superseded promise never hangs (and its caller's busy flag clears).
function settlePending() {
  if (active?.kind === 'confirm') active.resolve(false);
  else if (active) active.resolve(null);
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

  choose(
    message: string,
    opts: { title?: string; options: ChoiceOption[] },
  ): Promise<string | null> {
    return new Promise((resolve) => {
      settlePending();
      active = {
        kind: 'choose',
        title: opts.title ?? t('dialog.confirmTitle'),
        message,
        options: opts.options,
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

  resolveChoice(value: string | null): void {
    const a = active;
    active = null;
    if (a?.kind === 'choose') a.resolve(value);
  },
};
