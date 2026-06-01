<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import JobLog from './JobLog.svelte';

  let { packId }: { packId: string } = $props();

  let jobId = $state<string | null>(null);
  let busy = $state(false);
  let err = $state('');

  async function build() {
    busy = true;
    err = '';
    jobId = null;
    try {
      const { job_id } = await api.buildPack(packId);
      jobId = job_id;
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
      busy = false;
    }
  }
</script>

<div class="bc">
  <div class="bar row">
    <button class="primary" onclick={build} disabled={busy}>
      {busy ? 'building...' : 'Build pack'}
    </button>
  </div>
  <p class="muted hint">
    Loads the pack's config + curator, applies the curator chain, resolves
    sources, and publishes the manifest. Runs on the mirror; the log is live.
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
