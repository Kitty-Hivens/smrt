<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import type { ResolveReport } from '../lib/types';

  let { report }: { report: ResolveReport } = $props();

  // a clean run has nothing in any of the finding lists
  const clean = $derived(
    report.missing.length === 0 &&
      report.conflicts.length === 0 &&
      report.version_issues.length === 0 &&
      report.overlaps.length === 0 &&
      report.required_hints.length === 0 &&
      report.unresolved.length === 0,
  );
</script>

<div class="panel resrep">
  <div class="rhead">
    <span class="faint">{t('resolve.resolved', { n: report.resolved_mods, total: report.declared_mods })}</span>
    {#if report.missing.length}<span class="pill danger">{t('resolve.missing', { n: report.missing.length })}</span>{/if}
    {#if report.conflicts.length}<span class="pill danger">{t('resolve.conflicts', { n: report.conflicts.length })}</span>{/if}
    {#if report.version_issues.length}<span class="pill warn">{t('resolve.versionIssues', { n: report.version_issues.length })}</span>{/if}
    {#if report.overlaps.length}<span class="pill warn">{t('resolve.overlaps', { n: report.overlaps.length })}</span>{/if}
    {#if report.required_hints.length}<span class="pill info">{t('resolve.hints', { n: report.required_hints.length })}</span>{/if}
    {#if report.unresolved.length}<span class="pill faint">{t('resolve.unresolved', { n: report.unresolved.length })}</span>{/if}
    {#if clean}<span class="pill ok">{t('resolve.clean')}</span>{/if}
  </div>

  {#if report.missing.length}
    <div class="rlist">
      <div class="rl-h danger">{t('resolve.missingH')}</div>
      {#each report.missing as m}
        <div class="rl-row">
          <span class="mono strong">{m.target}</span>
          {#if m.version_range}<span class="mono faint">{m.version_range}</span>{/if}
          <span class="faint">{t('resolve.neededBy', { who: m.needed_by.join(', ') })}</span>
          <span class="src mono">{m.source}</span>
        </div>
      {/each}
    </div>
  {/if}

  {#if report.conflicts.length}
    <div class="rlist">
      <div class="rl-h danger">{t('resolve.conflictsH')}</div>
      {#each report.conflicts as c}
        <div class="rl-row">
          <span class="mono strong">{c.a}</span>
          <span class="faint">{c.breaks ? t('resolve.breaks') : t('resolve.conflictsWith')}</span>
          <span class="mono strong">{c.b}</span>
          <span class="src mono">{c.source}</span>
        </div>
      {/each}
    </div>
  {/if}

  {#if report.version_issues.length}
    <div class="rlist">
      <div class="rl-h warn">{t('resolve.versionIssuesH')}</div>
      {#each report.version_issues as v}
        <div class="rl-row">
          <span class="mono strong">{v.filename}</span>
          <span class="faint">{t('resolve.ships', { v: v.present_version })}</span>
          <span class="mono">{v.required_range}</span>
          <span class="faint">{t('resolve.neededBy', { who: v.needed_by.join(', ') })}</span>
        </div>
      {/each}
    </div>
  {/if}

  {#if report.overlaps.length}
    <div class="rlist">
      <div class="rl-h warn">{t('resolve.overlapsH')}</div>
      {#each report.overlaps as o}
        <div class="rl-row">
          <span class="mono strong">{o.capability}</span>
          <span class="faint">{o.mods.join(', ')}</span>
        </div>
      {/each}
    </div>
  {/if}

  {#if report.required_hints.length}
    <div class="rlist">
      <div class="rl-h info">{t('resolve.hintsH')}</div>
      {#each report.required_hints as h}
        <div class="rl-row">
          <span class="mono strong">{h.filename}</span>
          {#if h.modid}<span class="mono faint">{h.modid}</span>{/if}
          <span class="faint">{t('resolve.neededBy', { who: h.needed_by.join(', ') })}</span>
        </div>
      {/each}
    </div>
  {/if}

  {#if report.unresolved.length}
    <div class="rlist">
      <div class="rl-h faint">{t('resolve.unresolvedH')}</div>
      {#each report.unresolved as u}
        <div class="rl-row"><span class="mono">{u}</span></div>
      {/each}
    </div>
  {/if}

  {#if report.version_windows_unchecked}
    <div class="rfoot faint">{t('resolve.unchecked', { n: report.version_windows_unchecked })}</div>
  {/if}
</div>

<style>
  .resrep {
    padding: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .rhead {
    display: flex;
    gap: var(--space-2) var(--space-3);
    flex-wrap: wrap;
    align-items: center;
    font-size: 12px;
  }
  .pill {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
    border: 1px solid var(--seam);
    white-space: nowrap;
  }
  .pill.danger {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 40%, transparent);
    background: var(--danger-soft);
  }
  .pill.warn {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 40%, transparent);
    background: var(--warn-soft);
  }
  .pill.info {
    color: var(--info);
    border-color: color-mix(in srgb, var(--info) 40%, transparent);
  }
  .pill.ok {
    color: var(--ok);
    border-color: color-mix(in srgb, var(--ok) 40%, transparent);
  }
  .pill.faint {
    color: var(--fg-dim);
  }
  .rlist {
    margin-top: var(--space-3);
  }
  .rl-h {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 6px;
  }
  .rl-h.danger {
    color: var(--danger);
  }
  .rl-h.warn {
    color: var(--warn);
  }
  .rl-h.info {
    color: var(--info);
  }
  .rl-h.faint {
    color: var(--fg-dim);
  }
  .rl-row {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2) var(--space-3);
    font-size: 12px;
    padding: 3px 0;
    border-top: 1px solid var(--seam);
  }
  .rl-row:first-of-type {
    border-top: none;
  }
  .strong {
    font-weight: 600;
  }
  .src {
    margin-left: auto;
    font-size: 10px;
    color: var(--fg-dim);
    opacity: 0.7;
  }
  .rfoot {
    margin-top: var(--space-3);
    font-size: 11px;
  }
</style>
