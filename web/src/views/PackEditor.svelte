<script lang="ts">
  import { untrack } from 'svelte';
  import { flip } from 'svelte/animate';
  import { fade } from 'svelte/transition';
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';
  import { t } from '../lib/i18n.svelte';
  import { isDebug } from '../lib/roles';
  import type {
    DeclaredAsset,
    JobStatus,
    PackConfig,
    ResolveReport,
    SourceDecl,
    ValidateReport,
  } from '../lib/types';
  import BuildConsole from './BuildConsole.svelte';
  import BrandingEditor from './BrandingEditor.svelte';
  import PackGraph from './PackGraph.svelte';
  import JobLog from './JobLog.svelte';
  import ModIcon from './ModIcon.svelte';
  import ResolvePanel from './ResolvePanel.svelte';
  import ModrinthPicker from './ModrinthPicker.svelte';
  import MirrorPicker from './MirrorPicker.svelte';
  import GithubPicker from './GithubPicker.svelte';
  import PackPreview from './PackPreview.svelte';
  import DropZone from './ui/DropZone.svelte';
  import Field from './ui/Field.svelte';
  import Section from './ui/Section.svelte';
  import Select from './ui/Select.svelte';
  import TabStrip from './ui/TabStrip.svelte';

  const MOD_SOURCE_OPTIONS = [
    { value: 'smrt_cache', label: 'cache' },
    { value: 'modrinth', label: 'modrinth' },
    { value: 'smrt_static', label: 'static' },
  ];
  const ASSET_SOURCE_OPTIONS = [
    { value: 'smrt_static', label: 'static' },
    { value: 'modrinth', label: 'modrinth' },
    { value: 'smrt_cache', label: 'cache' },
  ];
  // The loaders the registry models via loader_parent, offered as a picker rather
  // than a free-text field. An unrecognised value already on a config (a loader we
  // don't list) is kept as its own option so editing never silently drops it.
  const KNOWN_LOADERS = ['forge', 'cleanroom', 'neoforge', 'fabric', 'quilt'];

  let {
    packId,
    onClose,
    me,
  }: { packId: string; onClose: () => void; me: { login: string } } = $props();

  // GitHub-style danger delete: type "<login>/<pack>" in a modal to confirm.
  async function deletePack() {
    const expected = `${me.login}/${packId.split('/').pop()}`;
    const typed = await dialogs.prompt(t('packs.deleteConfirm', { id: expected }), {
      title: t('packs.deleteTitle'),
      placeholder: expected,
    });
    if (typed == null) return;
    if (typed.trim() !== expected) {
      err = t('packs.deleteMismatch');
      return;
    }
    try {
      await api.deletePack(packId);
      onClose();
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
  }

  // debug operators can force an archival upload past the Modrinth-coverage gate
  let canDebug = $state(false);
  api
    .me()
    .then((m) => (canDebug = isDebug(m?.role)))
    .catch(() => {});

  // Upload a self-hosted jar for this community pack -- it enters the moderation
  // queue; once approved it is in the shared cache to add via "from mirror". The
  // uploader names the jar's upstream origin for archival provenance.
  async function onUploadJar(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    const maintainer = await dialogs.prompt(t('pe.uploadMaintainer'), {
      title: t('pe.uploadJar'),
      placeholder: t('pe.uploadMaintainerHint'),
    });
    if (maintainer == null) return; // cancelled
    const opts = { maintainer: maintainer.trim() || undefined };
    try {
      await api.uploadJar(packId, file, opts);
      await dialogs.confirm(t('pe.uploadQueued', { name: file.name }), {
        title: t('pe.uploadJar'),
      });
    } catch (x) {
      // A coverage rejection ("Modrinth already carries ...") can be forced only
      // by a debug operator -- the repackage-for-FML-handshake exception (#37/#44).
      const coverage =
        x instanceof ApiError && x.status === 400 && x.body.includes('already carries');
      if (canDebug && coverage) {
        const force = await dialogs.confirm(t('pe.uploadForce', { name: file.name }), {
          title: t('pe.uploadForceTitle'),
          danger: true,
        });
        if (force) {
          try {
            await api.uploadJar(packId, file, { ...opts, force: true });
            await dialogs.confirm(t('pe.uploadQueued', { name: file.name }), {
              title: t('pe.uploadJar'),
            });
          } catch (y) {
            err = y instanceof ApiError ? `${y.status} ${y.body}` : String(y);
          }
          return;
        }
      }
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    }
  }

  type Tab = 'config' | 'branding' | 'graph' | 'build';
  let tab = $state<Tab>('config');
  let previewOpen = $state(false);
  let previewToken = $state(0);

  // bootstrap-from-SC-archive (only shown when there is no config yet)
  let bootstrapMode = $state(false);
  let bootMc = $state('1.12.2');
  let bootLoader = $state('');
  let bootName = $state('');
  let bootBusy = $state(false);
  let bootJobId = $state<string | null>(null);

  // source picker: { src, row } -- row null means "add a new mod"
  let pick = $state<{ src: 'cache' | 'modrinth' | 'github'; row: number | null } | null>(null);
  // a resolve-report suggestion routed into the Modrinth picker as its search
  let suggestQuery = $state('');
  let dropBusy = $state(false);
  // asset Modrinth picker: which folder + Modrinth project kind
  let assetPick = $state<{ folder: string; projectType: 'resourcepack' | 'shader' } | null>(null);
  let assetDropBusy = $state(false);


  let cfg = $state<PackConfig | null>(null);
  let tagsStr = $state('');
  // pack-card gallery as newline-separated text, mirrored into cfg.pack_meta on save
  let cardGalleryStr = $state('');
  let loading = $state(true);
  let err = $state('');

  // autosave
  type SaveState = 'idle' | 'saving' | 'saved' | 'error';
  let saveState = $state<SaveState>('idle');
  let saveErr = $state('');
  // signature of the last-persisted state; autosave fires only when it differs,
  // which also keeps the initial load from triggering a spurious save
  let lastSig = '';
  let saveTimer: ReturnType<typeof setTimeout> | undefined;

  // validate the saved config against an uploaded SC archive
  let validating = $state(false);
  let valReport = $state<ValidateReport | null>(null);
  let valErr = $state('');

  // resolve the saved config against the registry dependency graph
  let resolving = $state(false);
  let resReport = $state<ResolveReport | null>(null);
  let resErr = $state('');

  // published build versions, for "revert config to build" (config edits autosave
  // with no undo, so the last built state is the recovery point). The picker is an
  // action menu -- `revertPick` resets to the placeholder after each choice.
  let revertVersions = $state<string[]>([]);
  let revertPick = $state('');
  const revertOptions = $derived(revertVersions.map((v) => ({ value: v, label: v })));

  async function load() {
    loading = true;
    err = '';
    try {
      const c = await api.packConfig(packId);
      if (!c.pack_meta) {
        c.pack_meta = { icon_url: null, banner_url: null, gallery_urls: [], description_md: null };
      }
      cfg = c;
      tagsStr = (c.tags ?? []).join(', ');
      cardGalleryStr = (c.pack_meta.gallery_urls ?? []).join('\n');
      lastSig = sig();
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        cfg = null; // offer to create
      } else {
        err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
      }
    }
    // best-effort: an unbuilt pack has no versions to revert to
    try {
      revertVersions = (await api.manifestVersions(packId)).builds.map((b) => b.version_number);
    } catch {
      revertVersions = [];
    }
    loading = false;
  }
  load();

  async function revertTo(version: string) {
    if (!version || !cfg) return;
    const ok = await dialogs.confirm(t('pe.revertConfirm', { version }), { danger: true });
    if (!ok) return;
    try {
      const c = await api.revertPackConfig(packId, version);
      if (!c.pack_meta) {
        c.pack_meta = { icon_url: null, banner_url: null, gallery_urls: [], description_md: null };
      }
      cfg = c;
      tagsStr = (c.tags ?? []).join(', ');
      cardGalleryStr = (c.pack_meta.gallery_urls ?? []).join('\n');
      lastSig = sig(); // matches new cfg -> autosave doesn't re-fire
      if (previewOpen) previewToken++;
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
  }

  function createBlank() {
    cfg = {
      pack_id: packId,
      display_name: packId,
      tagline: '',
      minecraft_version: '1.12.2',
      loader: { name: 'forge', version: '' },
      java_major: 8,
      tags: [],
      featured: false,
      mods: [],
      assets: [],
      pack_meta: { icon_url: null, banner_url: null, gallery_urls: [], description_md: null },
      // ownership + publication are server-authoritative; these are placeholders
      // the backend overwrites on create (owner = the creator) / preserves on edit.
      owner: 0,
      tier: 'official',
      visibility: 'published',
    };
    tagsStr = '';
    cardGalleryStr = '';
  }

  // content signature; the debounced autosave fires only when it changes. A JSON
  // array keeps the parts unambiguous (no separator a field value could forge).
  function sig(): string {
    return cfg ? JSON.stringify([$state.snapshot(cfg), tagsStr, cardGalleryStr]) : '';
  }

  // debounced autosave: deep-reads cfg + tags + gallery, persists once they settle
  $effect(() => {
    if (!cfg) return;
    const s = sig();
    if (s === lastSig) return;
    saveState = 'saving';
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => doSave(s), 700);
  });

  // a cleared text input holds "" -- normalize to null so an empty card field is
  // omitted from the published summary rather than serialized as ""
  const blankToNull = (v: string | null | undefined) => (v && v.trim() ? v.trim() : null);

  async function doSave(s: string) {
    if (!cfg) return;
    const snap = $state.snapshot(cfg);
    const payload: PackConfig = {
      ...snap,
      tags: tagsStr
        .split(',')
        .map((x) => x.trim())
        .filter(Boolean),
      pack_meta: {
        icon_url: blankToNull(snap.pack_meta.icon_url),
        banner_url: blankToNull(snap.pack_meta.banner_url),
        description_md: blankToNull(snap.pack_meta.description_md),
        gallery_urls: cardGalleryStr
          .split('\n')
          .map((x) => x.trim())
          .filter(Boolean),
      },
    };
    try {
      await api.savePackConfig(packId, payload);
      lastSig = s;
      saveState = 'saved';
      if (previewOpen) previewToken++; // auto-refresh the preview
    } catch (e) {
      saveState = 'error';
      saveErr = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
  }

  async function onValidate(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    validating = true;
    valErr = '';
    valReport = null;
    try {
      valReport = await api.validatePack(packId, file);
    } catch (x) {
      valErr = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    } finally {
      validating = false;
      input.value = '';
    }
  }

  // Resolve reads the SAVED config, so flush a pending autosave first -- the
  // report must reflect what is on screen, not the last debounced save.
  async function onResolve() {
    if (!cfg) return;
    resolving = true;
    resErr = '';
    try {
      const s = sig();
      if (s !== lastSig) {
        clearTimeout(saveTimer);
        await doSave(s);
        if (saveState === 'error') {
          resErr = saveErr;
          resReport = null;
          return;
        }
      }
      resReport = await api.resolvePack(packId);
    } catch (x) {
      resErr = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
      resReport = null;
    } finally {
      resolving = false;
    }
  }

  async function onBootstrap(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    bootBusy = true;
    err = '';
    bootJobId = null;
    try {
      const { job_id } = await api.bootstrapPack(
        packId,
        {
          minecraft_version: bootMc.trim(),
          loader_version: bootLoader.trim(),
          display_name: bootName.trim() || undefined,
        },
        file,
      );
      bootJobId = job_id;
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
      bootBusy = false;
    } finally {
      input.value = '';
    }
  }

  function onBootDone(status: JobStatus) {
    bootBusy = false;
    if (status === 'done') {
      bootstrapMode = false;
      load();
    }
  }

  // ── mods ──
  function blankSource(type: SourceDecl['type']): SourceDecl {
    if (type === 'modrinth') return { type, project_id: '', version_id: '' };
    if (type === 'smrt_cache') return { type, sha1: '' };
    return { type, rel_path: '' };
  }

  function changeSourceType(i: number, type: SourceDecl['type']) {
    cfg!.mods[i].source = blankSource(type);
  }

  function removeMod(i: number) {
    cfg!.mods = cfg!.mods.filter((_, j) => j !== i);
  }

  // Sticky sort: the list stays ordered as mods are added (an added mod slots
  // into place instead of landing at the end). Defaults to A-Z so a freshly
  // opened pack reads in order and an added mod lands where it belongs.
  let sortDir = $state<'asc' | 'desc' | null>('asc');

  // Re-sort only on a structural change (a mod added/removed -> length changes)
  // or a direction change. The sort itself runs untracked, so editing a mod's
  // filename does NOT re-trigger this -- the row won't jump out from under the
  // cursor mid-edit. Reassigns cfg.mods so autosave + the per-mod display table
  // (same list) follow.
  $effect(() => {
    const dir = sortDir;
    if (!cfg || !dir) return;
    void cfg.mods.length; // dependency: structural changes only
    untrack(() => {
      if (!cfg) return;
      const sign = dir === 'asc' ? 1 : -1;
      const sorted = [...cfg.mods].sort(
        (a, b) => a.filename.localeCompare(b.filename, undefined, { sensitivity: 'base' }) * sign,
      );
      if (sorted.some((m, i) => m !== cfg!.mods[i])) cfg.mods = sorted;
    });
  });

  async function onDropJars(files: File[]) {
    if (!cfg) return;
    dropBusy = true;
    err = '';
    try {
      for (const file of files) {
        if (!file.name.endsWith('.jar')) continue;
        const sha1 = await api.uploadCacheJar(file);
        cfg.mods = [
          ...cfg.mods,
          {
            filename: file.name,
            default_enabled: true,
            source: { type: 'smrt_cache', sha1 },
          },
        ];
      }
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    } finally {
      dropBusy = false;
    }
  }

  // a stable identity for a declared source, so the same artifact isn't added
  // twice (cache by sha1, Modrinth by project+version, static by path)
  function sourceKey(s: SourceDecl): string {
    if (s.type === 'smrt_cache') return `c:${s.sha1}`;
    if (s.type === 'modrinth') return `m:${s.project_id}:${s.version_id}`;
    return `s:${s.rel_path}`;
  }

  // a pick from the mirror carries the resolved source (cache when the mirror
  // holds the bytes, else Modrinth) plus the build's install flags when known
  type MirrorSel = {
    filename: string;
    source: SourceDecl;
    default_enabled?: boolean;
  };

  // append a declared mod unless an identical source is already present; returns
  // whether it was added
  function appendMod(sel: MirrorSel): boolean {
    if (!cfg) return false;
    const key = sourceKey(sel.source);
    if (cfg.mods.some((m) => sourceKey(m.source) === key)) return false;
    cfg.mods = [
      ...cfg.mods,
      {
        filename: sel.filename,
        default_enabled: sel.default_enabled ?? true,
        source: sel.source,
      },
    ];
    return true;
  }

  // single pick from the mirror: add a new row or re-point the row being edited,
  // then close
  function onMirrorPick(sel: MirrorSel) {
    if (!cfg || !pick) return;
    if (pick.row === null) {
      appendMod(sel);
    } else {
      const m = cfg.mods[pick.row];
      m.source = sel.source;
      if (!m.filename) m.filename = sel.filename;
    }
    pick = null;
  }

  // cherry-pick one mod from a build without closing -- keep adding from it
  function onMirrorAddOne(sel: MirrorSel) {
    appendMod(sel);
  }

  // re-add a whole build's mod set; preserves each mod's required/default flags,
  // skips artifacts already present, then closes
  function onMirrorAddMany(items: MirrorSel[]) {
    if (!cfg) return;
    for (const it of items) appendMod(it);
    pick = null;
  }

  // a GitHub ingest always lands a fresh jar in the cache -> a cache source
  function onGithubPick(sel: { sha1: string; filename: string }) {
    onMirrorPick({ filename: sel.filename, source: { type: 'smrt_cache', sha1: sel.sha1 } });
  }

  // pull an asset from a build (Builds tab) into this pack, deduped by dest; the
  // asset already carries its resolved source, so it is appended as-is
  function onMirrorAddAsset(a: DeclaredAsset) {
    if (!cfg) return;
    const assets = cfg.assets ?? [];
    if (assets.some((x) => x.dest === a.dest)) return;
    cfg.assets = [...assets, a];
  }

  function onModrinthPick(sel: { project_id: string; slug: string; version_id: string }) {
    if (!cfg || !pick) return;
    if (pick.row === null) {
      cfg.mods = [
        ...cfg.mods,
        {
          filename: `${sel.slug}.jar`,
          default_enabled: true,
          source: { type: 'modrinth', project_id: sel.project_id, version_id: sel.version_id },
        },
      ];
    } else {
      const m = cfg.mods[pick.row];
      m.source = { type: 'modrinth', project_id: sel.project_id, version_id: sel.version_id };
      if (!m.filename) m.filename = `${sel.slug}.jar`;
    }
    pick = null;
    suggestQuery = '';
  }

  function addAsset() {
    cfg!.assets = [
      ...(cfg!.assets ?? []),
      { dest: '', required: true, source: { type: 'smrt_static', rel_path: '' } },
    ];
  }
  function removeAsset(i: number) {
    cfg!.assets = (cfg!.assets ?? []).filter((_, j) => j !== i);
  }

  function onAssetModrinthPick(sel: { project_id: string; slug: string; version_id: string }) {
    if (!cfg || !assetPick) return;
    cfg.assets = [
      ...(cfg.assets ?? []),
      {
        dest: `${assetPick.folder}/${sel.slug}.zip`,
        required: true,
        source: { type: 'modrinth', project_id: sel.project_id, version_id: sel.version_id },
      },
    ];
    assetPick = null;
  }

  async function onDropAssets(files: File[]) {
    if (!cfg) return;
    assetDropBusy = true;
    err = '';
    try {
      for (const file of files) {
        const rel = `_nexira/assets/${file.name}`;
        await api.uploadStatic(packId, rel, file);
        cfg.assets = [
          ...(cfg.assets ?? []),
          { dest: file.name, required: true, source: { type: 'smrt_static', rel_path: rel } },
        ];
      }
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    } finally {
      assetDropBusy = false;
    }
  }

  const loaderOptions = $derived.by(() => {
    const cur = cfg?.loader.name?.trim();
    const names = cur && !KNOWN_LOADERS.includes(cur) ? [cur, ...KNOWN_LOADERS] : KNOWN_LOADERS;
    return names.map((l) => ({ value: l, label: l }));
  });

  const tabItems = $derived([
    { value: 'config', label: t('pe.tab.config') },
    { value: 'branding', label: t('pe.tab.branding') },
    { value: 'graph', label: t('pe.tab.graph') },
    { value: 'build', label: t('pe.tab.build') },
  ]);
</script>

<div class="hd">
  <h2 class="ttl mono">{packId}<span class="faint">/{t('pe.edit')}</span></h2>
  <TabStrip value={tab} tabs={tabItems} ariaLabel={t('pe.edit')} onChange={(v) => (tab = v as Tab)} />
  <div class="spacer"></div>
  {#if !loading && cfg && tab === 'config' && revertVersions.length}
    <span class="revertsel">
      <Select
        compact
        full
        bind:value={revertPick}
        options={revertOptions}
        placeholder={t('pe.revertPick')}
        title={t('pe.revertTo')}
        ariaLabel={t('pe.revertTo')}
        onChange={(v) => {
          if (v) revertTo(v);
          revertPick = '';
        }}
      />
    </span>
  {/if}
  {#if !loading && cfg && tab === 'config'}
    <span class="savestate" class:err={saveState === 'error'} title={saveErr}>
      {#if saveState === 'saving'}{t('pe.saving')}
      {:else if saveState === 'saved'}{t('pe.saved')}
      {:else if saveState === 'error'}{t('pe.saveError')}{/if}
    </span>
  {/if}
  {#if !loading && cfg}
    <button class="pv" class:active={previewOpen} onclick={() => (previewOpen = !previewOpen)}>
      {previewOpen ? t('pe.hidePreview') : t('pe.preview')}
    </button>
  {/if}
  <button onclick={onClose}>{t('common.close')}</button>
</div>

{#if err}<div class="err mono">{err}</div>{/if}

<div class="body" class:split={previewOpen}>
  <div class="editcol">
    {#if loading}
      <div class="muted mono">{t('common.loading')}</div>
    {:else if tab === 'config'}
      {#if !cfg}
        <div class="panel empty">
          <p class="muted">{t('pe.noConfig', { id: packId })}</p>
          <div class="opts">
            <button class="primary" onclick={createBlank}>{t('pe.createBlank')}</button>
            <button onclick={() => (bootstrapMode = !bootstrapMode)}>{t('pe.bootstrap')}</button>
          </div>
          {#if bootstrapMode}
            <div class="bootform">
              <div class="brow">
                <Field label={t('pe.mcVersion')}><input bind:value={bootMc} placeholder="1.12.2" /></Field>
                <Field label={t('pe.loaderVersion')}><input bind:value={bootLoader} placeholder="14.23.5.2922" /></Field>
                <Field label={t('pe.displayName')}><input bind:value={bootName} placeholder={packId} /></Field>
              </div>
              <label class="upbtn">
                {bootBusy ? t('pe.bootWorking') : t('pe.bootChoose')}
                <input
                  type="file"
                  accept=".zip"
                  onchange={onBootstrap}
                  disabled={bootBusy || !bootMc.trim() || !bootLoader.trim()}
                  hidden
                />
              </label>
              {#if bootJobId}{#key bootJobId}<JobLog jobId={bootJobId} onDone={onBootDone} />{/key}{/if}
            </div>
          {/if}
        </div>
      {:else}
        {#if resErr}<div class="err mono">{resErr}</div>{/if}
        {#if resReport}<ResolvePanel
            report={resReport}
            onSuggest={(sel) => {
              suggestQuery = sel.replace(/^modrinth:/, '');
              pick = { src: 'modrinth', row: null };
            }}
          />{/if}
        {#if valErr}<div class="err mono">{valErr}</div>{/if}
        {#if valReport}
          <div class="panel valrep">
            <div class="valhead">
              <span style="color:var(--ok)">{t('pe.valMatched', { n: valReport.matched })}</span>
              <span style={valReport.missing_in_config.length ? 'color:var(--danger)' : 'opacity:.62'}>
                {t('pe.valMissing', { n: valReport.missing_in_config.length })}
              </span>
              <span class="faint">{t('pe.valExtra', { n: valReport.extra_in_config.length })}</span>
              <span class="faint">{t('pe.valScMods', { n: valReport.sc_mod_count })}</span>
            </div>
            {#if valReport.missing_in_config.length}
              <div class="vallist">
                <div class="vl-h" style="color:var(--danger)">{t('pe.valMissingH')}</div>
                {#each valReport.missing_in_config as m}<div class="mono vl-row">{m}</div>{/each}
              </div>
            {/if}
            {#if valReport.extra_in_config.length}
              <div class="vallist">
                <div class="vl-h faint">{t('pe.valExtraH')}</div>
                {#each valReport.extra_in_config as m}<div class="mono vl-row">{m}</div>{/each}
              </div>
            {/if}
          </div>
        {/if}

        <Section title={t('pe.basics')}>
          <div class="meta">
            <Field label={t('pe.displayName')}><input bind:value={cfg.display_name} /></Field>
            <Field label={t('pe.mcVersion')}><input bind:value={cfg.minecraft_version} /></Field>
            <Field label={t('pe.loaderName')}>
              <Select full bind:value={cfg.loader.name} options={loaderOptions} ariaLabel={t('pe.loaderName')} />
            </Field>
            <Field label={t('pe.loaderVersion')}><input bind:value={cfg.loader.version} /></Field>
            <Field label={t('pe.java')}><input type="number" bind:value={cfg.java_major} /></Field>
            <label class="chk"><input type="checkbox" bind:checked={cfg.featured} /> {t('pe.featured')}</label>
            <Field label={t('pe.tagline')} wide><input bind:value={cfg.tagline} /></Field>
            <Field label={t('pe.tags')} hint={t('pe.tagsHint')} wide><input bind:value={tagsStr} /></Field>
          </div>
        </Section>

        <Section title={t('pe.mods')} count={cfg.mods.length}>
          {#snippet actions()}
            <button class="sm" class:active={sortDir === 'asc'} onclick={() => (sortDir = 'asc')} title={t('pe.sortHint')}>{t('pe.sortAsc')}</button>
            <button class="sm" class:active={sortDir === 'desc'} onclick={() => (sortDir = 'desc')} title={t('pe.sortHint')}>{t('pe.sortDesc')}</button>
            <button class="sm" onclick={() => (pick = { src: 'cache', row: null })}>{t('pe.fromMirror')}</button>
            <button class="sm" onclick={() => (pick = { src: 'modrinth', row: null })}>{t('pe.fromModrinth')}</button>
            <button class="sm" onclick={() => (pick = { src: 'github', row: null })}>{t('pe.fromGithub')}</button>
            {#if packId.startsWith('u/')}
              <label class="sm valbtn">
                {t('pe.uploadJar')}
                <input type="file" accept=".jar" onchange={onUploadJar} hidden />
              </label>
            {/if}
            <button class="sm" onclick={onResolve} disabled={resolving} title={t('resolve.hint')}>
              {resolving ? t('resolve.resolving') : t('resolve.resolve')}
            </button>
            <label class="sm valbtn">
              {validating ? t('pe.validating') : t('pe.validate')}
              <input type="file" accept=".zip" onchange={onValidate} disabled={validating} hidden />
            </label>
          {/snippet}

          <DropZone
            label={dropBusy ? t('pe.uploading') : t('pe.dropJars')}
            accept=".jar"
            busy={dropBusy}
            onFiles={onDropJars}
          />

          <div class="mods">
            {#each cfg.mods as m, i (m)}
              <div class="modrow" animate:flip={{ duration: 200 }} in:fade={{ duration: 180 }}>
                <ModIcon name={m.filename} iconUrl={m.display?.icon_url} source={m.source} size={24} mono />
                <input class="fn mono" bind:value={m.filename} placeholder={t('pe.filename')} />
                <span class="srcsel">
                  <Select
                    compact
                    full
                    value={m.source.type}
                    options={MOD_SOURCE_OPTIONS}
                    ariaLabel={t('pe.source')}
                    onChange={(v) => changeSourceType(i, v as SourceDecl['type'])}
                  />
                </span>
                <div class="ref">
                  {#if m.source.type === 'smrt_cache'}
                    <button class="sm" onclick={() => (pick = { src: 'cache', row: i })}>{t('pe.choose')}</button>
                    <span class="refval mono faint">{m.source.sha1 ? m.source.sha1.slice(0, 12) : t('pe.unset')}</span>
                  {:else if m.source.type === 'modrinth'}
                    <button class="sm" onclick={() => (pick = { src: 'modrinth', row: i })}>{t('pe.choose')}</button>
                    <span class="refval mono faint">{m.source.project_id || t('pe.unset')}</span>
                  {:else}
                    <input class="mono" bind:value={m.source.rel_path} placeholder="rel_path" />
                  {/if}
                </div>
                <label class="ck" title={t('pe.defHint')}><input type="checkbox" bind:checked={m.default_enabled} /> {t('pe.def')}</label>
                <input class="slug mono" bind:value={m.slug} placeholder={t('pe.slug')} title={t('pe.slugHint')} />
                <button class="danger sm del" onclick={() => removeMod(i)} aria-label={t('common.delete')}>x</button>
              </div>
            {/each}
            {#if cfg.mods.length === 0}
              <div class="muted empty-row">{t('pe.noMods')}</div>
            {/if}
          </div>
        </Section>

        <Section title={t('pe.assets')} count={(cfg.assets ?? []).length}>
          {#snippet actions()}
            <button class="sm" onclick={() => (assetPick = { folder: 'resourcepacks', projectType: 'resourcepack' })}>{t('pe.asset.resourcepack')}</button>
            <button class="sm" onclick={() => (assetPick = { folder: 'shaderpacks', projectType: 'shader' })}>{t('pe.asset.shader')}</button>
            <button class="sm" onclick={addAsset}>{t('pe.addAsset')}</button>
          {/snippet}
          <DropZone
            label={assetDropBusy ? t('pe.uploading') : t('pe.dropAssets')}
            busy={assetDropBusy}
            onFiles={onDropAssets}
          />
          <div class="panel scroll flushtable">
            <table>
              <thead>
                <tr>
                  <th>{t('pe.dest')}</th>
                  <th style="width:120px">{t('pe.source')}</th>
                  <th>{t('pe.ref')}</th>
                  <th style="width:60px">{t('pe.req')}</th>
                  <th style="width:44px"></th>
                </tr>
              </thead>
              <tbody>
                {#each cfg.assets ?? [] as a, i}
                  <tr>
                    <td><input class="mono" bind:value={a.dest} /></td>
                    <td>
                      <Select
                        compact
                        full
                        value={a.source.type}
                        options={ASSET_SOURCE_OPTIONS}
                        ariaLabel={t('pe.source')}
                        onChange={(v) => (cfg!.assets![i].source = blankSource(v as SourceDecl['type']))}
                      />
                    </td>
                    <td>
                      {#if a.source.type === 'modrinth'}
                        <input class="mono" bind:value={a.source.project_id} placeholder="project_id" />
                        <input class="mono" bind:value={a.source.version_id} placeholder="version_id" />
                      {:else if a.source.type === 'smrt_cache'}
                        <input class="mono" bind:value={a.source.sha1} placeholder="sha1" />
                      {:else}
                        <input class="mono" bind:value={a.source.rel_path} placeholder="rel_path" />
                      {/if}
                    </td>
                    <td class="ctr"><input type="checkbox" bind:checked={a.required} /></td>
                    <td class="ctr"><button class="danger sm" onclick={() => removeAsset(i)} aria-label={t('common.delete')}>x</button></td>
                  </tr>
                {/each}
                {#if (cfg.assets ?? []).length === 0}
                  <tr><td colspan="5" class="muted">{t('pe.noAssets')}</td></tr>
                {/if}
              </tbody>
            </table>
          </div>
        </Section>

        <Section title={t('pe.card.title')}>
          <div class="card">
            <Field label={t('pe.card.icon')} wide><input class="mono" bind:value={cfg.pack_meta.icon_url} placeholder="https://.../icon.png" /></Field>
            <Field label={t('pe.card.banner')} wide><input class="mono" bind:value={cfg.pack_meta.banner_url} placeholder="https://.../banner.png" /></Field>
            <Field label={t('pe.card.gallery')} wide><textarea class="mono" rows="3" bind:value={cardGalleryStr}></textarea></Field>
            <Field label={t('pe.card.description')} wide><textarea class="mono" rows="5" bind:value={cfg.pack_meta.description_md}></textarea></Field>
          </div>
        </Section>
      {/if}
    {:else if tab === 'branding'}
      <BrandingEditor {packId} />
      {#if cfg}
        <div class="dzone">
          <div class="dztitle mono">{t('pe.dangerZone')}</div>
          <div class="dzrow">
            <span class="dztext muted">{t('pe.deleteExplain')}</span>
            <button class="danger" onclick={deletePack}>{t('common.delete')}</button>
          </div>
        </div>
      {/if}
    {:else if tab === 'graph'}
      <PackGraph {packId} />
    {:else if tab === 'build'}
      <BuildConsole {packId} />
    {/if}
  </div>
  {#if previewOpen}
    <div class="previewcol">
      {#key previewToken}<PackPreview {packId} />{/key}
    </div>
  {/if}
</div>

{#if pick?.src === 'cache' && cfg}
  <MirrorPicker
    mc={cfg.minecraft_version}
    loader={cfg.loader.name}
    allowMany={pick.row === null}
    onClose={() => (pick = null)}
    onPick={onMirrorPick}
    onAddOne={onMirrorAddOne}
    onAddMany={onMirrorAddMany}
    onAddAsset={onMirrorAddAsset}
  />
{/if}
{#if pick?.src === 'modrinth' && cfg}
  <ModrinthPicker
    mc={cfg.minecraft_version}
    loader={cfg.loader.name}
    initialQuery={suggestQuery}
    onClose={() => {
      pick = null;
      suggestQuery = '';
    }}
    onPick={onModrinthPick}
  />
{/if}
{#if pick?.src === 'github' && cfg}
  <GithubPicker onClose={() => (pick = null)} onPick={onGithubPick} />
{/if}
{#if assetPick && cfg}
  <ModrinthPicker
    mc={cfg.minecraft_version}
    projectType={assetPick.projectType}
    onClose={() => (assetPick = null)}
    onPick={onAssetModrinthPick}
  />
{/if}

<style>
  .hd {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3) var(--space-4);
    margin-bottom: var(--space-4);
  }
  .ttl {
    font-size: 16px;
  }
  .spacer {
    flex: 1;
  }
  .savestate {
    font-size: 12px;
    color: var(--fg-dim);
    min-width: 78px;
    text-align: right;
  }
  .revertsel {
    display: inline-flex;
    max-width: 180px;
  }
  .savestate.err {
    color: var(--danger);
  }
  .err {
    color: var(--danger);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    font-size: 12px;
    margin-bottom: var(--space-3);
  }
  .empty {
    padding: var(--space-6);
    text-align: center;
  }
  .opts {
    display: flex;
    justify-content: center;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
  .meta {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-3) var(--space-4);
  }
  .card {
    display: grid;
    gap: var(--space-3) var(--space-4);
  }
  .card textarea {
    resize: vertical;
    width: 100%;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    align-self: end;
    padding-bottom: 8px;
  }
  .pv.active {
    border-color: var(--accent);
    color: var(--accent-strong);
  }
  button.sm {
    padding: 4px 10px;
    font-size: 12px;
  }
  button.sm.active {
    border-color: var(--accent);
    color: var(--accent-strong);
  }

  /* section spacing */
  .body :global(.section) {
    margin-bottom: var(--space-4);
  }

  /* mods */
  .mods {
    margin-top: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .modrow {
    display: grid;
    grid-template-columns: 24px minmax(120px, 1.4fr) 96px minmax(120px, 1.2fr) auto minmax(90px, 1fr) 30px;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--seam);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .modrow input {
    padding: 5px 7px;
    font-size: 12px;
  }
  /* curator slug in the 7th grid column: the stable optional-toggle key for
     smrt_cache mods (ADR 0002) */
  .modrow .slug {
    min-width: 0;
    opacity: 0.85;
  }
  /* the source-type Select wrapper occupies the grid's 3rd column; the trigger
     (full) fills it, and min-width:0 lets it shrink in the narrow flex reflow */
  .srcsel {
    display: flex;
    min-width: 0;
  }
  .ref {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .refval {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
  }
  .ck {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--fg-dim);
    white-space: nowrap;
  }
  .del {
    padding: 4px 8px;
  }
  .empty-row {
    padding: var(--space-3);
    font-size: 13px;
  }
  .flushtable {
    margin-top: var(--space-3);
  }
  td.ctr {
    text-align: center;
  }
  td input {
    padding: 5px 7px;
    font-size: 12px;
  }
  /* A file-input label styled as a button: the global `button` rules do not reach
     a <label>, and the `.sm` class only styles `button.sm`, so without this the
     control rendered as bare text next to real buttons. Matches `button.sm`. */
  .valbtn {
    display: inline-flex;
    align-items: center;
    font-family: var(--sans);
    font-size: 12px;
    font-weight: 600;
    color: var(--fg);
    background: var(--panel-2);
    border: 1px solid var(--seam-bright);
    border-radius: var(--radius-sm);
    padding: 4px 10px;
    cursor: pointer;
    transition:
      border-color 0.13s ease,
      background 0.13s ease;
  }
  .valbtn:hover {
    background: var(--panel-3);
  }
  .valrep {
    padding: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .valhead {
    display: flex;
    gap: var(--space-4);
    flex-wrap: wrap;
    font-size: 12px;
  }
  .vallist {
    margin-top: var(--space-3);
  }
  .vl-h {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 6px;
  }
  .vl-row {
    font-size: 12px;
    padding: 2px 0;
  }
  .bootform {
    margin-top: var(--space-4);
    text-align: left;
  }
  .brow {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-3);
    margin-bottom: var(--space-3);
  }
  .upbtn {
    display: inline-block;
    font-size: 13px;
    color: var(--fg);
    background: var(--panel-2);
    border: 1px solid var(--seam-bright);
    border-radius: var(--radius-sm);
    padding: 8px 14px;
    cursor: pointer;
  }
  .upbtn:hover {
    border-color: var(--accent);
  }
  .body.split {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--space-5);
    align-items: start;
  }
  .editcol {
    min-width: 0;
  }
  .previewcol {
    position: sticky;
    top: 12px;
    max-height: calc(100vh - 96px);
    overflow: auto;
    min-width: 0;
  }
  .dzone {
    margin-top: var(--space-6);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, var(--seam));
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .dztitle {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--danger);
    padding: var(--space-3) var(--space-4);
    background: var(--danger-soft);
    border-bottom: 1px solid color-mix(in srgb, var(--danger) 30%, var(--seam));
  }
  .dzrow {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-4);
  }
  .dztext {
    flex: 1;
    font-size: 12px;
  }

  /* ---- responsive reflow ---- */
  /* multi-column forms collapse; the editor/preview split stacks. The desktop
     rules above are left untouched, so wide layouts are unchanged. */
  @media (max-width: 768px) {
    .meta,
    .brow {
      grid-template-columns: repeat(2, 1fr);
    }
    .body.split {
      grid-template-columns: 1fr;
    }
    .previewcol {
      position: static;
      max-height: none;
    }
  }
  @media (max-width: 560px) {
    .meta,
    .brow {
      grid-template-columns: 1fr;
    }
  }

  /* mod row: the 8-column desktop grid becomes a stacked flex card on narrow
     viewports. Every control is preserved -- only the arrangement changes. */
  @media (min-width: 561px) and (max-width: 768px) {
    .modrow {
      display: flex;
      flex-wrap: wrap;
      gap: var(--space-2);
    }
    .modrow .fn {
      flex: 1 1 45%;
      min-width: 120px;
      width: auto;
    }
    .modrow .srcsel {
      flex: 0 0 auto;
      width: auto;
    }
    .modrow .ref {
      flex: 1 1 45%;
      min-width: 120px;
    }
    .modrow .ck {
      flex: 0 0 auto;
    }
    .modrow .slug {
      flex: 1 1 240px;
      min-width: 120px;
      width: auto;
    }
    .modrow .del {
      flex: 0 0 auto;
    }
  }
  @media (max-width: 560px) {
    .modrow {
      display: flex;
      flex-wrap: wrap;
      gap: var(--space-2);
    }
    .modrow .fn {
      flex: 1 1 auto;
      min-width: 0;
      width: auto;
    }
    .modrow .srcsel,
    .modrow .ref {
      flex: 1 1 100%;
      width: auto;
    }
    .modrow .ck {
      flex: 0 0 auto;
    }
    .modrow .slug {
      flex: 1 1 auto;
      min-width: 120px;
      width: auto;
    }
    .modrow .del {
      flex: 0 0 auto;
    }
  }
</style>
