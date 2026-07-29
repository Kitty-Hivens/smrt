<script lang="ts">
  import { api } from '../lib/api';
  import { notifyFail } from '../lib/toasts.svelte';
  import { t } from '../lib/i18n.svelte';
  import type { Commit, CommitStatus, JobStatus } from '../lib/types';
  import JobLog from './JobLog.svelte';
  import PackHistory from './PackHistory.svelte';

  let { packId, historyTick = 0 }: { packId: string; historyTick?: number } = $props();

  // The history, read here because a build is made from a commit (#122) -- the
  // state that decides whether the build button can do anything is the same
  // state the history shows.
  let status = $state<CommitStatus | null>(null);
  let log = $state<Commit[]>([]);

  async function refreshHistory() {
    try {
      [status, log] = await Promise.all([api.commitStatus(packId), api.commits(packId)]);
    } catch {
      // a pack with no history yet answers nothing useful; the console still works
    }
  }

  $effect(() => {
    // re-read when the pack changes, and when anyone in the pack commits
    void packId;
    void historyTick;
    void refreshHistory();
  });

  let jobId = $state<string | null>(null);
  let busy = $state(false);
  let packVersion = $state('');
  // publishing a release is an explicit act; the everyday build is a beta
  let channel = $state<'release' | 'beta' | 'alpha'>('beta');
  let changelog = $state('');
  // What the pre-publish check refused to publish over. Read from the job
  // rather than scraped out of the log, so the offer below only appears for a
  // refusal an override can actually answer.
  let blocked = $state<string[]>([]);

  async function build(overrideChecks = false, fromCommit?: string) {
    busy = true;
    jobId = null;
    blocked = [];
    try {
      const { job_id } = await api.buildPack(packId, {
        packVersion: packVersion.trim() || undefined,
        channel,
        changelog: changelog.trim() || undefined,
        overrideChecks,
        fromCommit,
      });
      jobId = job_id;
    } catch (e) {
      notifyFail(e);
      busy = false;
    }
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
  />
  <div class="bar">
    <button class="primary" onclick={() => build()} disabled={busy}>
      {busy ? t('bld.building') : t('bld.build')}
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
  <label class="notes">
    {t('bld.changelog')}
    <textarea rows="3" bind:value={changelog} placeholder={t('bld.changelogPlaceholder')}></textarea>
  </label>
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
