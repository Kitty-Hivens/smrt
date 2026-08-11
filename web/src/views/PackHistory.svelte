<script lang="ts">
  // A pack's history, where a build is decided (#122).
  //
  // This sits with the build rather than in a view of its own on purpose: a
  // build is made from a commit, so "what am I about to ship" and "what have I
  // checkpointed" are one question. Splitting them would let someone answer the
  // second in a place where the first is not on screen. A commit itself is a
  // place with an address of its own, which is where the whole of one is read.
  import { api } from '../lib/api';
  import { notifyFail, toasts } from '../lib/toasts.svelte';
  import { t } from '../lib/i18n.svelte';
  import { dialogs } from '../lib/dialogs.svelte';
  import { route, href, plainClick } from '../lib/route.svelte';
  import { suggest, tally } from '../lib/changes';
  import ChangeList from './ChangeList.svelte';
  import type { Commit, CommitLogEntry, CommitStatus, ConfigChange } from '../lib/types';

  let {
    packId,
    status,
    log,
    onChanged,
    onBuildCommit,
    busy = false,
    message = $bindable(''),
    body = $bindable(''),
  }: {
    packId: string;
    status: CommitStatus | null;
    log: CommitLogEntry[];
    // The history moved; the parent re-reads it and everyone else learns over
    // the pack's event stream.
    onChanged: () => void;
    onBuildCommit: (commitId: string) => void;
    busy?: boolean;
    // Owned by the build console, because the same sentence serves both acts:
    // committing on its own, and committing as the first half of a build.
    message?: string;
    body?: string;
  } = $props();

  let working = $state(false);

  // What a commit would record, straight from the mirror: the same diff the
  // build gate counts, so the number and the list can no longer disagree.
  const changes = $derived<ConfigChange[]>(status?.changes ?? []);
  const uncommitted = $derived(status?.uncommitted ?? 0);
  // Who has worked since the last checkpoint, so the commit names them before
  // it is pressed rather than after.
  const pending = $derived(status?.pending_authors ?? []);
  const counts = $derived(tally(changes));

  async function commit() {
    const text = full();
    if (!text) return;
    working = true;
    try {
      await api.commit(packId, text);
      message = '';
      body = '';
      onChanged();
    } catch (e) {
      notifyFail(e);
    } finally {
      working = false;
    }
  }

  /// Subject and body as one message, git's shape: a first line that fits in a
  /// log, a blank line, and whatever else is worth saying.
  function full(): string {
    const subject = message.trim();
    const rest = body.trim();
    if (!subject) return '';
    return rest ? `${subject}\n\n${rest}` : subject;
  }

  /// The suggestion, in the reader's language. Offered rather than filled in:
  /// a box that writes itself gets pressed without being read.
  const suggested = $derived.by(() => {
    const s = suggest(changes);
    if (!s) return '';
    if (s.kind === 'mixed') {
      const parts = [];
      if (s.counts.add) parts.push(t('chg.suggest.nAdded', { n: s.counts.add }));
      if (s.counts.remove) parts.push(t('chg.suggest.nRemoved', { n: s.counts.remove }));
      if (s.counts.change) parts.push(t('chg.suggest.nUpdated', { n: s.counts.change }));
      return parts.join(', ');
    }
    return t(`chg.suggest.${s.kind}`, { what: s.what.join(', ') });
  });

  async function restore(entry: CommitLogEntry) {
    working = true;
    try {
      // What a restore would do, before it does it: the commit read against the
      // working state. Pressing "restore" used to be a single click with no
      // statement of consequences anywhere on the way.
      let summary = '';
      try {
        const diff = await api.commitDiff(packId, entry.id, 'live');
        const c = tally(diff.changes);
        summary = diff.changes.length
          ? t('hist.restoreEffect', { add: c.add, remove: c.remove, change: c.change })
          : t('hist.restoreNoop');
      } catch {
        // an unreadable diff must not block the act; the question still names
        // the commit it is about to put back
        summary = t('hist.restoreUnknown');
      }
      const ok = await dialogs.confirm(
        `${t('hist.restoreAsk', { id: short(entry.id), message: entry.message })}\n\n${summary}`,
        { title: t('hist.restore'), danger: true },
      );
      if (!ok) return;
      await api.restoreCommit(packId, entry.id);
      toasts.push({ kind: 'ok', text: t('hist.restored', { id: short(entry.id) }) });
      onChanged();
    } catch (e) {
      notifyFail(e);
    } finally {
      working = false;
    }
  }

  // version_id -> its version_number, filled in behind the rows. A pin on
  // screen has to read like a version, and only Modrinth knows what
  // `bqMxf6Ua` is called.
  let labels = $state<Record<string, string>>({});

  $effect(() => {
    void loadLabels(changes);
  });

  /// One lookup per project that actually moved, and only for rows naming one.
  async function loadLabels(current: ConfigChange[]) {
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

  const short = (id: string) => id.slice(0, 8);

  async function copyId(id: string) {
    try {
      await navigator.clipboard.writeText(id);
      toasts.push({ kind: 'ok', text: t('hist.idCopied') });
    } catch {
      // a browser that refuses the clipboard still shows the id in the title
    }
  }

  // The timestamp as a person reads it. The stored value is RFC 3339 UTC; what
  // is useful on screen is when it was, locally.
  function when(at: string): string {
    const d = new Date(at);
    return Number.isNaN(d.getTime()) ? at : d.toLocaleString();
  }

  function openCommit(e: MouseEvent, c: Commit) {
    if (!plainClick(e)) return;
    e.preventDefault();
    route.openCommit(packId, c.id);
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
      <span class="muted mono" title={status.head.id}>{short(status.head.id)}</span>
    {:else}
      <span class="dirty">{t('hist.none')}</span>
    {/if}
  </div>

  <ChangeList rows={changes} {labels} dense />

  <div class="declare">
    <div class="lines">
      <input
        bind:value={message}
        placeholder={suggested || t('hist.messagePlaceholder')}
        disabled={working || busy}
        onkeydown={(e) => {
          if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) commit();
        }}
      />
      <textarea
        bind:value={body}
        rows="2"
        placeholder={t('hist.bodyPlaceholder')}
        disabled={working || busy}
        onkeydown={(e) => {
          if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) commit();
        }}
      ></textarea>
    </div>
    <div class="acts">
      <button onclick={commit} disabled={working || busy || !message.trim()}>
        {t('hist.commit')}
      </button>
      {#if suggested && !message.trim()}
        <button class="link" onclick={() => (message = suggested)} disabled={working || busy}>
          {t('hist.useSuggested')}
        </button>
      {/if}
    </div>
  </div>
  <p class="muted hint">{t('hist.hint')}</p>

  {#if log.length}
    <ol class="log">
      {#each log as c (c.id)}
        <li>
          <div class="line">
            <a
              class="msg"
              href={href.commit(packId, c.id)}
              onclick={(e) => openCommit(e, c)}
              title={t('hist.openCommit')}
            >
              {c.message.split('\n')[0]}
            </a>
            <button class="id mono" title={c.id} onclick={() => copyId(c.id)}>
              {short(c.id)}
            </button>
          </div>
          <div class="meta muted">
            <span>{c.author}</span>
            <span>{when(c.at)}</span>
            {#if c.contributors.length > 1}
              <span>{t('hist.with', { who: c.contributors.slice(1).join(', ') })}</span>
            {/if}
            {#if c.builds.length}
              <span class="built" title={t('hist.builtFrom')}>{c.builds.join(', ')}</span>
            {:else}
              <span class="unbuilt">{t('hist.neverBuilt')}</span>
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
  {:else if status?.head}
    <!-- a head with no log means the read failed, not that nothing was ever
         declared: saying so beats an empty space where a history was -->
    <p class="muted empty">{t('hist.logUnread')}</p>
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
  .declare {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin-top: 10px;
  }
  .lines {
    display: flex;
    flex-direction: column;
    gap: 5px;
    flex: 1 1 auto;
  }
  .declare input,
  .declare textarea {
    font: inherit;
    width: 100%;
  }
  .declare textarea {
    resize: vertical;
  }
  .acts {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: flex-start;
  }
  .hint {
    font-size: var(--fs-sm);
    margin: 8px 0 0;
  }
  .empty {
    font-size: var(--fs-sm);
    margin: 14px 0 0;
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
    justify-content: space-between;
  }
  .msg {
    overflow-wrap: anywhere;
    color: inherit;
    text-decoration: none;
  }
  .msg:hover {
    text-decoration: underline;
  }
  .id {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--fg-dim);
    font-size: var(--fs-sm);
    cursor: pointer;
    flex: 0 0 auto;
  }
  .id:hover {
    color: var(--fg);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    font-size: var(--fs-sm);
    margin-top: 3px;
  }
  .built {
    color: var(--ok, var(--fg-dim));
    font-variant-numeric: tabular-nums;
  }
  .unbuilt {
    opacity: 0.7;
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
  @container view (max-width: 560px) {
    .declare {
      flex-direction: column;
    }
    .acts {
      flex-direction: row;
      align-items: center;
    }
  }
</style>
