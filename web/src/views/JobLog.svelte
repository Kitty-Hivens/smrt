<script lang="ts">
  import { api } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import type { JobStatus } from '../lib/types';

  let { jobId, onDone }: { jobId: string; onDone?: (status: JobStatus) => void } = $props();

  let lines = $state<string[]>([]);
  let status = $state<JobStatus>('running');
  const statusLabel = $derived(
    t(status === 'done' ? 'job.done' : status === 'failed' ? 'job.failed' : 'job.running'),
  );

  $effect(() => {
    // Re-subscribe when jobId changes; reset for the new job.
    lines = [];
    status = 'running';
    const source = new EventSource(api.jobEventsUrl(jobId));
    const finish = (s: JobStatus) => {
      status = s;
      source.close();
      onDone?.(s);
    };
    source.addEventListener('line', (ev) => {
      lines = [...lines, (ev as MessageEvent).data];
    });
    source.addEventListener('done', () => finish('done'));
    source.addEventListener('failed', () => finish('failed'));
    source.onerror = () => {
      if (status === 'running') {
        lines = [...lines, t('job.interrupted')];
        finish('failed');
      }
    };
    return () => source.close();
  });
</script>

<div class="jl">
  <span class="st mono" class:ok={status === 'done'} class:bad={status === 'failed'}>{statusLabel}</span>
  {#if lines.length}
    <pre class="log mono">{lines.join('\n')}</pre>
  {/if}
</div>

<style>
  .jl {
    margin-top: 4px;
  }
  .st {
    font-size: 12px;
    color: var(--fg-dim);
  }
  .st.ok {
    color: var(--ok);
  }
  .st.bad {
    color: var(--danger);
  }
  .log {
    background: var(--bg);
    border: 1px solid var(--seam);
    padding: 14px;
    margin: 10px 0 0;
    font-size: 12.5px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 460px;
    overflow: auto;
  }
</style>
