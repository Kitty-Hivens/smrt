<script lang="ts">
  import { api } from '../lib/api';
  import { notifyFail, toasts } from '../lib/toasts.svelte';
  import { t, LOCALES, type Locale } from '../lib/i18n.svelte';
  import type { CommitLogEntry, CommitStatus, JobStatus } from '../lib/types';
  import JobLog from './JobLog.svelte';
  import PackHistory from './PackHistory.svelte';

  let {
    packId,
    historyTick = 0,
    buildFrom = null,
    onBuildStarted = () => {},
  }: {
    packId: string;
    historyTick?: number;
    // A commit page asked for this build. The console owns building, so the
    // request arrives here rather than being made twice in two places.
    buildFrom?: string | null;
    onBuildStarted?: () => void;
  } = $props();

  // The history, read here because a build is made from a commit (#122) -- the
  // state that decides whether the build button can do anything is the same
  // state the history shows.
  let status = $state<CommitStatus | null>(null);
  let log = $state<CommitLogEntry[]>([]);
  // Where the next page of the log starts, or null at the end of the history.
  let logNext = $state<string | null>(null);
  let logFailed = $state(false);

  async function refreshHistory() {
    try {
      const [s, page] = await Promise.all([api.commitStatus(packId), api.commits(packId)]);
      status = s;
      log = page.rows;
      logNext = page.next;
      logFailed = false;
    } catch {
      // a pack with no history yet answers nothing useful; the console still
      // works, and the history view says it could not read rather than
      // pretending the pack has none
      logFailed = true;
    }
  }

  /// The next page, appended. Reading further back is a step someone takes, not
  /// something the editor does on its own on a pack with hundreds of them.
  async function moreHistory() {
    if (!logNext) return;
    try {
      const page = await api.commitsPage(logNext);
      log = [...log, ...page.rows];
      logNext = page.next;
    } catch (e) {
      notifyFail(e);
    }
  }

  $effect(() => {
    // re-read when the pack changes, and when anyone in the pack commits
    void packId;
    void historyTick;
    void refreshHistory();
  });

  // A build asked for from a commit page: the console is where a build happens,
  // so the request lands here and the log below shows it. Clearing the request
  // first keeps the effect from re-firing on its own build.
  $effect(() => {
    if (!buildFrom) return;
    const id = buildFrom;
    onBuildStarted();
    void build(false, id);
  });

  let jobId = $state<string | null>(null);
  let busy = $state(false);
  let packVersion = $state('');
  // publishing a release is an explicit act; the everyday build is a beta
  let channel = $state<'release' | 'beta' | 'alpha'>('beta');
  // Release notes per language rather than one box. The launcher renders them
  // to the player, and a mirror serving one community in its own language is
  // the ordinary case -- the languages offered are the panel's own, and the
  // wire accepts any tag.
  let notes = $state<Record<string, string>>(Object.fromEntries(LOCALES.map((l) => [l, ''])));
  let noteLang = $state<Locale>(LOCALES[0]);
  // What the pre-publish check refused to publish over. Read from the job
  // rather than scraped out of the log, so the offer below only appears for a
  // refusal an override can actually answer.
  let blocked = $state<string[]>([]);
  // The commit message, held here rather than in the history view: a build with
  // uncommitted work declares the checkpoint itself, so the same sentence is
  // the one the button uses.
  let commitMessage = $state('');
  // The rest of the message, under the subject line -- git's shape, and the
  // room a curator needs to say why rather than only what.
  let commitBody = $state('');

  // Whether the next publish has to declare a checkpoint first -- work sitting
  // uncommitted, or a pack that has never committed at all.
  const needsCommit = $derived(!status?.head || (status?.uncommitted ?? 0) > 0);

  async function build(overrideChecks = false, fromCommit?: string) {
    // Committing is the first half of building, not a hoop before it. The
    // mirror refuses a publish that has uncommitted work -- that refusal is the
    // honest answer for anything driving the API, but nobody should have to
    // meet it here and press the same button twice.
    if (!fromCommit && needsCommit) {
      const subject = commitMessage.trim();
      if (!subject) {
        toasts.push({ kind: 'info', text: t('bld.needsMessage') });
        return;
      }
      const rest = commitBody.trim();
      busy = true;
      try {
        await api.commit(packId, rest ? `${subject}\n\n${rest}` : subject);
        commitMessage = '';
        commitBody = '';
        await refreshHistory();
      } catch (e) {
        notifyFail(e);
        busy = false;
        return;
      }
    }
    busy = true;
    jobId = null;
    blocked = [];
    try {
      const { job_id } = await api.buildPack(packId, {
        packVersion: packVersion.trim() || undefined,
        channel,
        changelogI18n: written(),
        overrideChecks,
        fromCommit,
      });
      jobId = job_id;
    } catch (e) {
      notifyFail(e);
      busy = false;
    }
  }

  // Only the languages actually written. An empty box is a language the curator
  // skipped, not an empty release note.
  function written(): Record<string, string> | undefined {
    const out = Object.fromEntries(
      Object.entries(notes)
        .map(([k, v]) => [k, v.trim()])
        .filter(([, v]) => v),
    );
    return Object.keys(out).length ? out : undefined;
  }

  async function finished(jobStatus: JobStatus) {
    busy = false;
    // A build that landed published a commit's state; the history view says
    // which, so it is re-read rather than left showing the state before.
    void refreshHistory();
    if (jobStatus !== 'failed' || !jobId) return;
    try {
      blocked = (await api.jobStatus(jobId)).blocked ?? [];
    } catch {
      // the log already carries the findings; the offer is the only casualty
    }
  }
</script>

<div class="bc">
  <PackHistory
    {packId}
    {status}
    {log}
    {busy}
    onChanged={refreshHistory}
    onBuildCommit={(id) => build(false, id)}
    hasMore={!!logNext}
    failed={logFailed}
    onMore={moreHistory}
    bind:message={commitMessage}
    bind:body={commitBody}
  />
  <div class="bar">
    <button class="primary" onclick={() => build()} disabled={busy}>
      {busy ? t('bld.building') : needsCommit ? t('bld.commitAndBuild') : t('bld.build')}
    </button>
    <label class="ver">
      {t('bld.version')}
      <input class="mono" bind:value={packVersion} placeholder={t('bld.versionPlaceholder')} />
    </label>
    <label class="ver">
      {t('bld.channel')}
      <select bind:value={channel}>
        <option value="beta">beta</option>
        <option value="release">release</option>
        <option value="alpha">alpha</option>
      </select>
    </label>
  </div>
  <div class="notes">
    <div class="notes-h">
      <span>{t('bld.changelog')}</span>
      <div class="langs">
        {#each LOCALES as l (l)}
          <button
            class="lang"
            class:on={noteLang === l}
            onclick={() => (noteLang = l)}
            title={notes[l]?.trim() ? t('bld.langWritten') : t('bld.langEmpty')}
          >
            {l}{notes[l]?.trim() ? '' : ' ·'}
          </button>
        {/each}
      </div>
    </div>
    {#each LOCALES as l (l)}
      {#if noteLang === l}
        <textarea rows="3" bind:value={notes[l]} placeholder={t('bld.changelogPlaceholder')}
        ></textarea>
      {/if}
    {/each}
  </div>
  <p class="muted hint">{t('bld.hint')}</p>
  {#if jobId}
    {#key jobId}
      <JobLog {jobId} onDone={finished} />
    {/key}
  {/if}
  {#if blocked.length}
    <div class="gate">
      <h4>{t('bld.blocked')}</h4>
      <ul>
        {#each blocked as line (line)}
          <li>{line}</li>
        {/each}
      </ul>
      <p class="muted">{t('bld.overrideHint')}</p>
      <button onclick={() => build(true)} disabled={busy}>{t('bld.override')}</button>
    </div>
  {/if}
</div>

<style>
  .bc {
    padding: 4px 0;
  }
  .bar {
    display: flex;
    align-items: flex-end;
    gap: 14px;
  }
  .ver {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: var(--fs-sm);
    color: var(--fg-dim);
  }
  .ver input {
    width: 180px;
  }
  .notes {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: var(--fs-sm);
    color: var(--fg-dim);
    margin-top: 12px;
    max-width: 640px;
  }
  .notes-h {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
  }
  .langs {
    display: flex;
    gap: 4px;
  }
  .lang {
    background: none;
    border: 1px solid transparent;
    padding: 1px 7px;
    font: inherit;
    color: var(--fg-dim);
    cursor: pointer;
  }
  .lang.on {
    border-color: var(--line);
    color: var(--fg);
  }
  .notes textarea {
    resize: vertical;
    font: inherit;
  }
  .hint {
    font-size: var(--fs-sm);
    margin: 10px 0 14px;
    max-width: 640px;
  }
  .gate {
    border: 1px solid var(--danger);
    border-left-width: 3px;
    padding: 12px 16px;
    margin-top: 16px;
    max-width: 640px;
  }
  .gate h4 {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--danger);
  }
  .gate ul {
    margin: 8px 0;
    padding-left: 18px;
    font-size: var(--fs-sm);
    line-height: 1.6;
  }
  .gate p {
    font-size: var(--fs-sm);
    margin: 0 0 10px;
  }
  @container view (max-width: 560px) {
    .bar {
      flex-wrap: wrap;
    }
    .ver {
      flex: 1 1 100%;
    }
    .ver input {
      width: 100%;
    }
  }
</style>
