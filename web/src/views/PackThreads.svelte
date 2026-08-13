<script lang="ts">
  // What is being said about this pack: reports and proposals in one list.
  //
  // They are one list because they are one thing to a reader -- somebody asking
  // the pack's keepers for something -- and because a proposal that hid in its
  // own tab would be a request nobody stumbles over.
  import { api } from '../lib/api';
  import { notifyFail } from '../lib/toasts.svelte';
  import { t } from '../lib/i18n.svelte';
  import { route, href, plainClick } from '../lib/route.svelte';
  import type { Thread } from '../lib/types';

  let { packId, tick = 0 }: { packId: string; tick?: number } = $props();

  let rows = $state<Thread[]>([]);
  let loading = $state(true);
  let failed = $state(false);
  let showAll = $state(false);
  let working = $state(false);

  // The report form, folded away until wanted: the list is what people come for.
  let opening = $state(false);
  let title = $state('');
  let body = $state('');

  $effect(() => {
    void packId;
    void tick;
    void showAll;
    void load();
  });

  async function load() {
    loading = true;
    try {
      rows = await api.threads(packId, undefined, showAll);
      failed = false;
    } catch (e) {
      failed = true;
      notifyFail(e);
    } finally {
      loading = false;
    }
  }

  async function report() {
    const heading = title.trim();
    if (!heading) return;
    working = true;
    try {
      const opened = await api.openIssue(packId, heading, body.trim());
      title = '';
      body = '';
      opening = false;
      route.openThread(packId, opened.id);
    } catch (e) {
      notifyFail(e);
    } finally {
      working = false;
    }
  }

  function open(e: MouseEvent, row: Thread) {
    if (!plainClick(e)) return;
    e.preventDefault();
    route.openThread(packId, row.id);
  }

  function when(at: number): string {
    const d = new Date(at * 1000);
    return Number.isNaN(d.getTime()) ? String(at) : d.toLocaleDateString();
  }
</script>

<div class="threads">
  <div class="bar">
    <button class="link" onclick={() => (showAll = !showAll)}>
      {showAll ? t('thr.showOpen') : t('thr.showAll')}
    </button>
    <button class="link" onclick={() => (opening = !opening)}>{t('thr.report')}</button>
  </div>

  {#if opening}
    <div class="form">
      <input bind:value={title} placeholder={t('thr.titlePlaceholder')} disabled={working} />
      <textarea rows="3" bind:value={body} placeholder={t('thr.bodyPlaceholder')} disabled={working}
      ></textarea>
      <button onclick={report} disabled={working || !title.trim()}>{t('thr.send')}</button>
    </div>
  {/if}

  {#if loading && !rows.length}
    <p class="muted">{t('common.loading')}</p>
  {:else if failed}
    <p class="muted">{t('thr.unreadable')}</p>
  {:else if !rows.length}
    <p class="muted empty">{showAll ? t('thr.noneAtAll') : t('thr.noneOpen')}</p>
  {:else}
    <ol class="list">
      {#each rows as r (r.id)}
        <li>
          <div class="line">
            <span class="kind" data-kind={r.kind}>{t(`thr.kind.${r.kind}` as 'thr.kind.issue')}</span>
            <a href={href.thread(packId, r.id)} onclick={(e) => open(e, r)}>{r.title}</a>
            <span class="status" data-status={r.status}>{t(`thr.status.${r.status}` as 'thr.status.open')}</span>
          </div>
          <div class="meta muted">
            <span>#{r.id}</span>
            <span>{r.by_login ?? t('acc.unknownUser', { uid: r.by_uid })}</span>
            <span>{when(r.created_at)}</span>
            {#if r.comments}
              <span>{t('thr.comments', { n: r.comments })}</span>
            {/if}
          </div>
        </li>
      {/each}
    </ol>
  {/if}
</div>

<style>
  .threads {
    max-width: 720px;
    padding: 4px 0;
  }
  .bar {
    display: flex;
    gap: 14px;
    margin-bottom: 10px;
    font-size: var(--fs-sm);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 14px;
    max-width: 640px;
  }
  .form input,
  .form textarea {
    font: inherit;
    resize: vertical;
  }
  .form button {
    align-self: flex-start;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--line);
  }
  .list li {
    padding: 8px 0;
    border-bottom: 1px solid var(--line);
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .line a {
    color: inherit;
    text-decoration: none;
    overflow-wrap: anywhere;
  }
  .line a:hover {
    text-decoration: underline;
  }
  .kind {
    font-size: var(--fs-sm);
    font-variant: small-caps;
    color: var(--fg-dim);
  }
  .kind[data-kind='proposal'] {
    color: var(--accent, var(--fg));
  }
  .status {
    margin-left: auto;
    font-size: var(--fs-sm);
    color: var(--fg-dim);
  }
  .status[data-status='open'] {
    color: var(--ok, var(--fg));
  }
  .status[data-status='merged'] {
    color: var(--accent, var(--fg));
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    font-size: var(--fs-sm);
    margin-top: 3px;
  }
  .empty {
    font-size: var(--fs-sm);
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--accent, var(--fg));
    cursor: pointer;
  }
</style>
