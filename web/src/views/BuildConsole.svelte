<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import JobLog from './JobLog.svelte';

  let { packId }: { packId: string } = $props();

  let jobId = $state<string | null>(null);
  let busy = $state(false);
  let err = $state('');
  let packVersion = $state('');
  // publishing a release is an explicit act; the everyday build is a beta
  let channel = $state<'release' | 'beta' | 'alpha'>('beta');
  let changelog = $state('');

  async function build() {
    busy = true;
    err = '';
    jobId = null;
    try {
      const { job_id } = await api.buildPack(packId, {
        packVersion: packVersion.trim() || undefined,
        channel,
        changelog: changelog.trim() || undefined,
      });
      jobId = job_id;
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
      busy = false;
    }
  }
</script>

<div class="bc">
  <div class="bar">
    <button class="primary" onclick={build} disabled={busy}>
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
  {#if err}<div class="err mono">{err}</div>{/if}
  {#if jobId}
    {#key jobId}
      <JobLog {jobId} onDone={() => (busy = false)} />
    {/key}
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
    font-size: 12px;
    color: var(--fg-dim);
  }
  .ver input {
    width: 180px;
  }
  .notes {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--fg-dim);
    margin-top: 12px;
    max-width: 640px;
  }
  .notes textarea {
    resize: vertical;
    font: inherit;
  }
  .hint {
    font-size: 12px;
    margin: 10px 0 14px;
    max-width: 640px;
  }
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-bottom: 10px;
  }
  @media (max-width: 560px) {
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
