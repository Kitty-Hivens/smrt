<script lang="ts">
  // Declaring a checkpoint: what has changed since the last one, and the
  // sentence that names this one.
  //
  // It sits with the build rather than in a view of its own on purpose: a build
  // is made from a commit, so "what am I about to ship" and "what am I
  // checkpointing" are one question, and the same sentence serves both acts.
  // The list of past checkpoints is a different question -- one consulted while
  // working -- and lives in a dock of its own (PackLog). A commit itself is a
  // place with an address, which is where the whole of one is read.
  import { api } from '../lib/api';
  import { notifyFail, toasts } from '../lib/toasts.svelte';
  import { t } from '../lib/i18n.svelte';
  import { dialogs } from '../lib/dialogs.svelte';
  import { suggest } from '../lib/changes';
  import ChangeList from './ChangeList.svelte';
  import type { CommitStatus, ConfigChange } from '../lib/types';

  let {
    packId,
    status,
    onChanged,
    busy = false,
    working = $bindable(false),
    message = $bindable(''),
    body = $bindable(''),
  }: {
    packId: string;
    status: CommitStatus | null;
    // The history moved; the parent re-reads it and everyone else learns over
    // the pack's event stream.
    onChanged: () => void;
    busy?: boolean;
    /// Committing here. Bound out so the console's build button -- which commits
    /// too -- is not live at the same time.
    working?: boolean;
    // Owned by the build console, because the same sentence serves both acts:
    // committing on its own, and committing as the first half of a build.
    message?: string;
    body?: string;
  } = $props();

  // What a commit would record, straight from the mirror: the same diff the
  // build gate counts, so the number and the list can no longer disagree.
  const changes = $derived<ConfigChange[]>(status?.changes ?? []);
  const uncommitted = $derived(status?.uncommitted ?? 0);
  // Who has worked since the last checkpoint, so the commit names them before
  // it is pressed rather than after.
  const pending = $derived(status?.pending_authors ?? []);
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


  // version_id -> its version_number, filled in behind the rows. A pin on
  // screen has to read like a version, and only Modrinth knows what
  // `bqMxf6Ua` is called.
  let labels = $state<Record<string, string>>({});

  $effect(() => {
    void loadLabels(changes);
  });

  /// One lookup per project that actually moved, and only for rows naming one.
  const asked = new Set<string>();

  async function loadLabels(current: ConfigChange[]) {
    // Once per project, not once per project per refresh: a pack event
    // re-reads the status, and every re-read used to re-ask Modrinth for every
    // moved pin on screen.
    const projects = [...new Set(current.map((r) => r.project).filter((p): p is string => !!p))]
      .filter((project) => !asked.has(project));
    for (const project of projects) {
      asked.add(project);
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


  // The timestamp as a person reads it. The stored value is RFC 3339 UTC; what
  // is useful on screen is when it was, locally.

</script>

<div class="hist">
  <div class="state">
    {#if !status?.head}
      <span class="dirty">{t('hist.none')}</span>
    {:else if uncommitted > 0}
      <span class="dirty">{t('hist.uncommitted', { n: uncommitted })}</span>
      {#if pending.length}
        <span class="muted">{t('hist.by', { who: pending.join(', ') })}</span>
      {/if}
    {:else if status?.head}
      <span class="clean">{t('hist.clean')}</span>
      <span class="muted mono" title={status.head.id}>{short(status.head.id)}</span>
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
