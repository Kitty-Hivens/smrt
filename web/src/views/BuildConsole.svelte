<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import type { JobStatus } from '../lib/types';

  let { packId }: { packId: string } = $props();

  let lines = $state<string[]>([]);
  let status = $state<JobStatus | 'idle'>('idle');
  let es: EventSource | null = null;

  async function build() {
    es?.close();
    lines = [];
    status = 'running';
    let jobId: string;
    try {
      ({ job_id: jobId } = await api.buildPack(packId));
    } catch (e) {
      status = 'failed';
      lines = [e instanceof ApiError ? `${e.status} ${e.body}` : String(e)];
      return;
    }
    const source = new EventSource(api.jobEventsUrl(jobId));
    es = source;
    source.addEventListener('line', (ev) => {
      lines = [...lines, (ev as MessageEvent).data];
    });
    source.addEventListener('done', () => {
      status = 'done';
      source.close();
    });
    source.addEventListener('failed', () => {
      status = 'failed';
      source.close();
    });
    source.onerror = () => {
      // Server closes the stream after the terminal event; only treat as an
      // error if we never reached a terminal state.
      if (status === 'running') {
        status = 'failed';
        lines = [...lines, '(log stream interrupted)'];
      }
      source.close();
    };
  }

  $effect(() => () => es?.close());
</script>

<div class="bc">
  <div class="bar row">
    <button class="primary" onclick={build} disabled={status === 'running'}>
      {status === 'running' ? 'building...' : 'Build pack'}
    </button>
    {#if status !== 'idle'}
      <span class="st mono" class:ok={status === 'done'} class:bad={status === 'failed'}>{status}</span>
    {/if}
  </div>
  <p class="muted hint">
    Loads the pack's config + curator, applies the curator chain, resolves
    sources, and publishes the manifest. Runs on the mirror; the log is live.
  </p>
  {#if lines.length}
    <pre class="log mono">{lines.join('\n')}</pre>
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
    font-size: 12.5px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 460px;
    overflow: auto;
    margin: 0;
  }
</style>
