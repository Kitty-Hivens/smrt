<script lang="ts">
  // A pack's history, where a build is decided (#122).
  //
  // This sits with the build rather than in a view of its own on purpose: a
  // build is made from a commit, so "what am I about to ship" and "what have I
  // checkpointed" are one question. Splitting them would let someone answer the
  // second in a place where the first is not on screen.
  import { api } from '../lib/api';
  import { notifyFail, toasts } from '../lib/toasts.svelte';
  import { t } from '../lib/i18n.svelte';
  import { diffConfigs, type ChangeRow } from '../lib/configdiff';
  import type { Commit, CommitStatus } from '../lib/types';

  let {
    packId,
    status,
    log,
    onChanged,
    onBuildCommit,
    busy = false,
    message = $bindable(''),
  }: {
    packId: string;
    status: CommitStatus | null;
    log: Commit[];
    // The history moved; the parent re-reads it and everyone else learns over
    // the pack's event stream.
    onChanged: () => void;
    onBuildCommit: (commitId: string) => void;
    busy?: boolean;
    // Owned by the build console, because the same sentence serves both acts:
    // committing on its own, and committing as the first half of a build.
    message?: string;
  } = $props();

  let working = $state(false);

  const uncommitted = $derived(status?.uncommitted ?? 0);
  // Who has worked since the last checkpoint, so the commit names them before
  // it is pressed rather than after.
  const pending = $derived(status?.pending_authors ?? []);

  async function commit() {
    const text = message.trim();
    if (!text) return;
    working = true;
    try {
      await api.commit(packId, text);
      message = '';
      onChanged();
    } catch (e) {
      notifyFail(e);
    } finally {
      working = false;
    }
  }

  async function restore(c: Commit) {
    working = true;
    try {
      await api.restoreCommit(packId, c.id);
      toasts.push({ kind: 'ok', text: t('hist.restored', { id: short(c.id) }) });
      onChanged();
    } catch (e) {
      notifyFail(e);
    } finally {
      working = false;
    }
  }

  // What is about to be checkpointed. Read from the two configs rather than
  // from the count beside it: the count is changed JSON paths, which is not a
  // list of things anyone did.
  let rows = $state<ChangeRow[]>([]);
  // version_id -> its version_number, filled in behind the rows. A pin on
  // screen has to read like a version, and only Modrinth knows what
  // `bqMxf6Ua` is called.
  let labels = $state<Record<string, string>>({});
  // Whether the two configs were actually read. Without it an empty list means
  // both "nothing a person did" and "the read has not landed yet".
  let pendingRead = $state(false);

  $effect(() => {
    const head = status?.head?.id;
    const pending = status?.uncommitted ?? 0;
    if (!head || pending === 0) {
      rows = [];
      pendingRead = false;
      return;
    }
    void loadChanges(head);
  });

  async function loadChanges(head: string) {
    try {
      const [committed, live] = await Promise.all([
        api.commitConfig(packId, head),
        api.packConfig(packId),
      ]);
      rows = diffConfigs(committed, live.config);
      pendingRead = true;
      void loadLabels(rows);
    } catch {
      // The list is an aid, not the act: a pack whose history cannot be read
      // still commits, and the count above still says something moved.
      rows = [];
      pendingRead = false;
    }
  }

  /// One lookup per project that actually moved, and only for rows naming one.
  async function loadLabels(current: ChangeRow[]) {
    const projects = [...new Set(current.map((r) => r.project).filter((p): p is string => !!p))];
    for (const project of projects) {
      try {
        const versions = await api.modrinthVersions(project);
        const found: Record<string, string> = {};
        for (const v of versions) found[v.id] = v.version_number;
        labels = { ...labels, ...found };
      } catch {
        // an unreachable Modrinth leaves the ids on screen, which still says
        // that the pin moved and to what
      }
    }
  }

  const label = (pin?: string) => (pin ? (labels[pin] ?? pin) : '');

  const short = (id: string) => id.slice(0, 8);

  // The timestamp as a person reads it. The stored value is RFC 3339 UTC; what
  // is useful on screen is when it was, locally.
  function when(at: string): string {
    const d = new Date(at);
    return Number.isNaN(d.getTime()) ? at : d.toLocaleString();
  }
</script>

<div class="hist">
  <div class="state">
    {#if uncommitted > 0}
      <span class="dirty">{t('hist.uncommitted', { n: uncommitted })}</span>
      {#if pending.length}
        <span class="muted">{t('hist.by', { who: pending.join(', ') })}</span>
      {/if}
    {:else if status?.head}
      <span class="clean">{t('hist.clean')}</span>
      <span class="muted mono">{short(status.head.id)}</span>
    {:else}
      <span class="dirty">{t('hist.none')}</span>
    {/if}
  </div>

  {#if pendingRead && !rows.length && uncommitted > 0}
    <!-- The count is changed JSON paths; a save the dependency fill touched
         moves plenty of them without anyone having done anything. Saying so
         beats an empty list under a number that says otherwise. -->
    <p class="muted derived">{t('hist.derivedOnly')}</p>
  {/if}

  {#if rows.length}
    <ul class="changes">
      {#each rows as r (r.group + r.op + r.label)}
        <li class={r.op}>
          <span class="sign">{r.op === 'add' ? '+' : r.op === 'remove' ? '-' : '~'}</span>
          <span class="what mono">{r.label}</span>
          {#if r.op === 'change' && r.from !== undefined && r.to !== undefined}
            <span class="move mono">{label(r.from)} &rarr; {label(r.to)}</span>
          {:else if r.op === 'change'}
            <span class="move muted">{t('hist.edited')}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  <div class="declare">
    <input
      bind:value={message}
      placeholder={t('hist.messagePlaceholder')}
      disabled={working || busy}
      onkeydown={(e) => e.key === 'Enter' && commit()}
    />
    <button onclick={commit} disabled={working || busy || !message.trim()}>
      {t('hist.commit')}
    </button>
  </div>
  <p class="muted hint">{t('hist.hint')}</p>

  {#if log.length}
    <ol class="log">
      {#each log as c (c.id)}
        <li>
          <div class="line">
            <span class="mono id">{short(c.id)}</span>
            <span class="msg">{c.message}</span>
          </div>
          <div class="meta muted">
            <span>{c.author}</span>
            <span>{when(c.at)}</span>
            {#if c.contributors.length > 1}
              <span>{t('hist.with', { who: c.contributors.slice(1).join(', ') })}</span>
            {/if}
            <button class="link" onclick={() => onBuildCommit(c.id)} disabled={busy || working}>
              {t('hist.buildThis')}
            </button>
            <button class="link" onclick={() => restore(c)} disabled={busy || working}>
              {t('hist.restore')}
            </button>
          </div>
        </li>
      {/each}
    </ol>
  {/if}
</div>

<style>
  .hist {
    max-width: 640px;
    margin-bottom: 18px;
  }
  .state {
    display: flex;
    gap: 10px;
    align-items: baseline;
    font-size: var(--fs-sm);
    margin-bottom: 8px;
  }
  .dirty {
    color: var(--warn, var(--fg));
  }
  .clean {
    color: var(--fg-dim);
  }
  .derived {
    font-size: var(--fs-sm);
    margin: 0 0 8px;
  }
  .changes {
    list-style: none;
    margin: 0 0 10px;
    padding: 0;
    font-size: var(--fs-sm);
    max-height: 190px;
    overflow-y: auto;
  }
  .changes li {
    display: flex;
    gap: 8px;
    align-items: baseline;
    padding: 1px 0;
  }
  .sign {
    width: 1ch;
    color: var(--fg-dim);
  }
  .changes li.add .sign {
    color: var(--ok, var(--fg));
  }
  .changes li.remove .sign {
    color: var(--danger, var(--fg));
  }
  .what {
    overflow-wrap: anywhere;
  }
  .move {
    color: var(--fg-dim);
  }
  .declare {
    display: flex;
    gap: 8px;
  }
  .declare input {
    flex: 1 1 auto;
    font: inherit;
  }
  .hint {
    font-size: var(--fs-sm);
    margin: 8px 0 0;
  }
  .log {
    list-style: none;
    margin: 14px 0 0;
    padding: 0;
    border-top: 1px solid var(--line);
  }
  .log li {
    padding: 8px 0;
    border-bottom: 1px solid var(--line);
  }
  .line {
    display: flex;
    gap: 10px;
    align-items: baseline;
  }
  .id {
    color: var(--fg-dim);
    font-size: var(--fs-sm);
  }
  .msg {
    overflow-wrap: anywhere;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    font-size: var(--fs-sm);
    margin-top: 3px;
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--accent, var(--fg));
    cursor: pointer;
  }
  .link:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
