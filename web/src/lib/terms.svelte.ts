// Rules-of-use gate: a member must accept before authoring or forking community
// content. `accepted` is seeded from /v1/me; `ensure()` shows the rules modal and
// records acceptance the first time, so later actions pass through silently.
import { api } from './api';
import { dialogs } from './dialogs.svelte';
import { t } from './i18n.svelte';

let accepted = $state(false);

export const terms = {
  init(v: boolean) {
    accepted = v;
  },
  get accepted() {
    return accepted;
  },
  async ensure(): Promise<boolean> {
    if (accepted) return true;
    const ok = await dialogs.confirm(t('terms.body'), { title: t('terms.title') });
    if (!ok) return false;
    await api.acceptTerms();
    accepted = true;
    return true;
  },
};
