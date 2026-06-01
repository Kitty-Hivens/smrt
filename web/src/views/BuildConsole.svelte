<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import JobLog from './JobLog.svelte';

  let { packId }: { packId: string } = $props();

  let jobId = $state<string | null>(null);
  let busy = $state(false);
  let err = $state('');
  let packVersion = $state('');

  async function build() {
    busy = true;
    err = '';
    jobId = null;
    try {
      const { job_id } = await api.buildPack(packId, {
        packVersion: packVersion.trim() || undefined,
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
      {busy ? 'building...' : 'Build pack'}
    </button>
    <label class="ver">
      pack_version
      <input class="mono" bind:value={packVersion} placeholder="(today's date)" />
    </label>
  </div>
  <p class="muted hint">
    Loads the pack's config + curator, applies the curator chain, resolves
    sources, and publishes the manifest. Runs on the mirror; the log is live.
    Leave pack_version blank for today's UTC date slug.
  </p>
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
</style>
