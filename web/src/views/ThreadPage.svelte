<script lang="ts">
  // One discussion: what was asked, what was said, and what was decided.
  //
  // A proposal shows the same page plus what taking it would do to this pack as
  // it stands now -- the review question is "what happens to my pack if I take
  // this", and the answer moves as the pack moves.
  import { api } from '../lib/api';
  import { notifyFail, toasts } from '../lib/toasts.svelte';
  import { t } from '../lib/i18n.svelte';
  import { dialogs } from '../lib/dialogs.svelte';
  import { route } from '../lib/route.svelte';
  import { tally } from '../lib/changes';
  import ChangeList from './ChangeList.svelte';
  import type { CommitDiff, ThreadView } from '../lib/types';

  let {
    packId,
    threadId,
    canEdit = false,
    onChanged,
  }: {
    packId: string;
    threadId: number;
    /// Whether this viewer keeps the pack: closes, merges, moderates.
    canEdit?: boolean;
    onChanged: () => void;
  } = $props();

  let view = $state<ThreadView | null>(null);
  let diff = $state<CommitDiff | null>(null);
  let loading = $state(true);
  let failed = $state(false);
  let working = $state(false);
  let reply = $state('');

  const thread = $derived(view?.thread ?? null);
  const isProposal = $derived(thread?.kind === 'proposal');
  const isOpen = $derived(thread?.status === 'open');
  const counts = $derived(tally(diff?.changes ?? []));

  $effect(() => {
    void load(threadId);
  });

  async function load(id: number) {
    loading = true;
    try {
      view = await api.thread(id);
      failed = false;
      // The diff is the reviewer's half of a proposal; an issue has none, and a
      // settled proposal's offer is history rather than a question.
      diff = view.thread.kind === 'proposal' ? await api.threadDiff(id).catch(() => null) : null;
    } catch (e) {
      failed = true;
      notifyFail(e);
    } finally {
      loading = false;
    }
  }

  async function act(run: () => Promise<unknown>) {
    working = true;
    try {
      await run();
      await load(threadId);
      onChanged();
    } catch (e) {
      notifyFail(e);
    } finally {
      working = false;
    }
  }

  async function say() {
    const body = reply.trim();
    if (!body) return;
    await act(async () => {
      await api.comment(threadId, body);
      reply = '';
    });
  }

  async function merge() {
    const c = counts;
    const ok = await dialogs.confirm(
      t('thr.mergeAsk', { add: c.add, remove: c.remove, change: c.change }),
      { title: t('thr.merge') },
    );
    if (!ok) return;
    await act(async () => {
      await api.mergeProposal(threadId);
      toasts.push({ kind: 'ok', text: t('thr.merged') });
    });
  }

  async function hide(commentId: number, hidden: boolean) {
    if (hidden && !(await dialogs.confirm(t('thr.hideAsk'), { title: t('thr.hide'), danger: true }))) {
      return;
    }
    await act(() => api.hideComment(commentId, hidden));
  }

  /// Whose words these are. Uid 0 is the mirror's own break-glass hand rather
  /// than a person, and "uid 0" on screen reads like a bug.
  function who(uid: number, login?: string | null): string {
    if (login) return login;
    return uid === 0 ? t('thr.byOperator') : t('acc.unknownUser', { uid });
  }

  function when(at: number): string {
    const d = new Date(at * 1000);
    return Number.isNaN(d.getTime()) ? String(at) : d.toLocaleString();
  }
</script>

<div class="page">
  <div class="top">
    <button class="link" onclick={() => route.closeThread()}>&larr; {t('thr.back')}</button>
  </div>

  {#if loading && !view}
    <p class="muted">{t('common.loading')}</p>
  {:else if failed && !view}
    <p class="muted">{t('thr.unreadable')}</p>
  {:else if thread}
    <header>
      <h2>{thread.title}</h2>
      <div class="meta muted">
        <span class="kind" data-kind={thread.kind}>{t(`thr.kind.${thread.kind}` as 'thr.kind.issue')}</span>
        <span class="status" data-status={thread.status}>{t(`thr.status.${thread.status}` as 'thr.status.open')}</span>
        <span>#{thread.id}</span>
        <span class="name">{who(thread.by_uid, thread.by_login)}</span>
        <span>{when(thread.created_at)}</span>
      </div>
      {#if thread.body}
        <p class="body">{thread.body}</p>
      {/if}
      <hr />
      {#if thread.merged_commit}
        <p class="muted small">
          {t('thr.mergedAs', { commit: thread.merged_commit.slice(0, 8) })}
        </p>
      {/if}
    </header>

    {#if isProposal}
      <section class="offer">
        <h3>{t('thr.offers')}</h3>
        {#if diff && diff.changes.length}
          <p class="muted small">
            {t('thr.offersLead', { add: counts.add, remove: counts.remove, change: counts.change })}
          </p>
          <ChangeList rows={diff.changes} />
        {:else if diff}
          <p class="muted small">{t('thr.offersNothing')}</p>
        {:else}
          <p class="muted small">{t('thr.offersUnreadable')}</p>
        {/if}
      </section>
    {/if}

    <section class="talk">
      {#each view?.comments ?? [] as c (c.id)}
        <article class:hidden={c.hidden}>
          <div class="who muted">
            <span class="name">{who(c.by_uid, c.by_login)}</span>
            <span>{when(c.created_at)}</span>
            {#if canEdit}
              <button class="link small" onclick={() => hide(c.id, !c.hidden)} disabled={working}>
                {c.hidden ? t('thr.show') : t('thr.hide')}
              </button>
            {/if}
          </div>
          {#if c.hidden}
            <p class="muted taken">{t('thr.taken')}</p>
          {:else}
            <p class="said">{c.body}</p>
          {/if}
        </article>
      {/each}

      <div class="say">
        <textarea
          rows="3"
          bind:value={reply}
          placeholder={t('thr.replyPlaceholder')}
          disabled={working}
          onkeydown={(e) => {
            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) say();
          }}
        ></textarea>
        <button onclick={say} disabled={working || !reply.trim()}>{t('thr.reply')}</button>
      </div>
    </section>

    <div class="acts">
      {#if isProposal && isOpen && canEdit}
        <button class="primary" onclick={merge} disabled={working}>{t('thr.merge')}</button>
      {/if}
      {#if isOpen}
        <button onclick={() => act(() => api.closeThread(threadId))} disabled={working}>
          {isProposal ? t('thr.decline') : t('thr.close')}
        </button>
      {:else if thread.kind === 'issue'}
        <button onclick={() => act(() => api.reopenThread(threadId))} disabled={working}>
          {t('thr.reopen')}
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .page {
    padding: 4px 0 20px;
    max-width: 720px;
  }
  .top {
    margin-bottom: 12px;
  }
  h2 {
    margin: 0 0 6px;
    font-size: var(--fs-lg, 1.1rem);
    overflow-wrap: anywhere;
  }
  h3 {
    margin: 0 0 6px;
    font-size: var(--fs-sm);
    color: var(--fg-dim);
    font-weight: 500;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    align-items: baseline;
    font-size: var(--fs-sm);
  }
  .kind {
    font-variant: small-caps;
  }
  .kind[data-kind='proposal'] {
    color: var(--accent, var(--fg));
  }
  .status[data-status='open'] {
    color: var(--ok, var(--fg));
  }
  .status[data-status='merged'] {
    color: var(--accent, var(--fg));
  }
  hr {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 14px 0 0;
  }
  .body {
    margin: 8px 0 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .small {
    font-size: var(--fs-sm);
  }
  .offer {
    margin: 18px 0;
    padding: 10px 12px;
    border: 1px solid var(--line);
  }
  .talk {
    margin-top: 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .talk article {
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-left-width: 2px;
  }
  .talk article.hidden {
    border-left-color: var(--danger, var(--line));
    opacity: 0.75;
  }
  .who {
    display: flex;
    gap: 12px;
    align-items: baseline;
    font-size: var(--fs-sm);
  }
  .who .name {
    color: var(--fg);
    font-weight: 500;
  }
  .who .link {
    margin-left: auto;
  }
  .said {
    margin: 4px 0 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .taken {
    margin: 4px 0 0;
    font-size: var(--fs-sm);
    font-style: italic;
  }
  .say {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: flex-start;
    padding: 12px 0;
  }
  .say textarea {
    font: inherit;
    resize: vertical;
    width: 100%;
    max-width: 640px;
  }
  .acts {
    display: flex;
    gap: 10px;
    margin-top: 8px;
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--accent, var(--fg));
    cursor: pointer;
  }
  .link.small {
    font-size: var(--fs-sm);
  }
  .link:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
