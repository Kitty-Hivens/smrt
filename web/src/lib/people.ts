// What to call somebody on screen.
//
// The mirror stores a GitHub uid and joins the login where it knows one, which
// leaves two cases a view must not render literally: an account that has never
// signed in here (there is no name to show, only the number), and uid 0, which
// is the mirror's own break-glass hand rather than a person. Three views were
// each writing this out, and the third of them printed a bare `0` where a name
// belongs.

import { t } from './i18n.svelte';

export function nameOf(uid: number, login?: string | null): string {
  if (login) return login;
  return uid === 0 ? t('common.operator') : t('acc.unknownUser', { uid });
}
