<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';
  import { t } from '../lib/i18n.svelte';
  import type { UploadRow } from '../lib/types';

  // The operator's moderation queue: pending member jar uploads. Approve promotes
  // the jar into the shared cache; reject drops the staged jar with a note.
  let uploads = $state<UploadRow[]>([]);
  let err = $state('');
  let loading = $state(true);

  async function load() {
    loading = true;
    err = '';
    try {
      uploads = await api.pendingUploads();
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      loading = false;
    }
  }
  load();

  async function approve(u: UploadRow) {
    try {
      await api.approveUpload(u.id);
      await load();
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
  }

  async function reject(u: UploadRow) {
    const note = await dialogs.prompt(t('mod.rejectNote'), { title: t('mod.reject') });
    if (note == null) return;
    try {
      await api.rejectUpload(u.id, note);
      await load();
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
  }

  const size = (n: number) =>
    n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1e3))} KB`;
</script>

<div class="view">
  {#if err}<div class="err mono">{err}</div>{/if}

  <div class="panel list">
    {#each uploads as u (u.id)}
      <div class="row">
        <div class="col">
          <div class="nm">{u.filename}</div>
          <div class="mm faint mono">
            {u.pack_id} &middot; {size(u.size_bytes)} &middot; uid {u.uploader}
          </div>
          <div class="sha faint mono">{u.sha1}</div>
        </div>
        <div class="grow"></div>
        <button class="ok" onclick={() => approve(u)}>{t('mod.approve')}</button>
        <button class="danger" onclick={() => reject(u)}>{t('mod.reject')}</button>
      </div>
    {/each}
    {#if uploads.length === 0 && !loading}
      <div class="empty muted">{t('mod.empty')}</div>
    {/if}
  </div>
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .err {
    color: var(--danger);
    font-size: 12px;
  }
  .list {
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    border-bottom: 1px solid var(--seam);
  }
  .row:last-child {
    border-bottom: none;
  }
  .col {
    min-width: 0;
  }
  .nm {
    font-size: 14px;
    font-weight: 600;
  }
  .mm {
    font-size: 11px;
    margin-top: 2px;
  }
  .sha {
    font-size: 10.5px;
    margin-top: 1px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .grow {
    flex: 1;
  }
  .ok {
    flex-shrink: 0;
    padding: 5px 12px;
    font-size: 12px;
    color: var(--ok);
  }
  .ok:hover {
    border-color: var(--ok);
  }
  .danger {
    flex-shrink: 0;
    padding: 5px 12px;
    font-size: 12px;
  }
  .empty {
    padding: var(--space-4);
    font-size: 12px;
  }
</style>
