<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { t } from '../lib/i18n.svelte';

  // Crop a dropped raster image to a fixed aspect, client-side. A fixed frame of
  // the target aspect sits over the image; the image pans (drag) and zooms
  // (slider) behind it, and Apply rasterises the framed region to a blob. No
  // backend or spec change -- the upload is just the cropped result.
  let {
    file,
    aspect,
    title,
    maxOut = 1024,
    onApply,
    onCancel,
  }: {
    file: File;
    aspect: number;
    title: string;
    maxOut?: number; // longest output edge, px (no upscaling past the source)
    onApply: (blob: Blob, ext: string) => void;
    onCancel: () => void;
  } = $props();

  let url = $state('');
  $effect(() => {
    const u = URL.createObjectURL(file);
    url = u;
    return () => URL.revokeObjectURL(u);
  });

  // frame size: fit the target aspect into a 520x360 box
  const MAXW = 520,
    MAXH = 360;
  const FW = $derived(aspect >= MAXW / MAXH ? MAXW : MAXH * aspect);
  const FH = $derived(aspect >= MAXW / MAXH ? MAXW / aspect : MAXH);

  let img = $state<HTMLImageElement | null>(null);
  let ready = $state(false);
  let nat = { w: 0, h: 0 };
  let cover = 1;

  let zoom = $state(1);
  let scale = $state(1);
  let tx = $state(0);
  let ty = $state(0);
  let busy = $state(false);
  let loadErr = $state(false);

  function clampPan() {
    tx = Math.min(0, Math.max(FW - nat.w * scale, tx));
    ty = Math.min(0, Math.max(FH - nat.h * scale, ty));
  }

  function onImgLoad() {
    if (!img) return;
    nat = { w: img.naturalWidth, h: img.naturalHeight };
    cover = Math.max(FW / nat.w, FH / nat.h);
    zoom = 1;
    scale = cover;
    tx = (FW - nat.w * scale) / 2;
    ty = (FH - nat.h * scale) / 2;
    ready = true;
  }

  function onZoom(z: number) {
    const cx = (FW / 2 - tx) / scale;
    const cy = (FH / 2 - ty) / scale;
    zoom = z;
    scale = cover * zoom;
    tx = FW / 2 - cx * scale;
    ty = FH / 2 - cy * scale;
    clampPan();
  }

  // drag to pan
  let dragging = false;
  let start = { x: 0, y: 0, tx: 0, ty: 0 };
  function onDown(e: PointerEvent) {
    if (!ready) return;
    dragging = true;
    start = { x: e.clientX, y: e.clientY, tx, ty };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!dragging) return;
    tx = start.tx + (e.clientX - start.x);
    ty = start.ty + (e.clientY - start.y);
    clampPan();
  }
  function onUp(e: PointerEvent) {
    dragging = false;
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
  }

  function apply() {
    if (!img || !ready) return;
    busy = true;
    const sw = FW / scale;
    const sh = FH / scale;
    const sx = -tx / scale;
    const sy = -ty / scale;
    // derive height from the rounded width so the output keeps the exact aspect
    // and never collapses to a zero dimension on a tiny crop
    const cw = Math.max(1, Math.round(Math.min(sw, maxOut)));
    const ch = Math.max(1, Math.round(cw / aspect));
    const canvas = document.createElement('canvas');
    canvas.width = cw;
    canvas.height = ch;
    const ctx = canvas.getContext('2d');
    if (!ctx) {
      busy = false;
      return;
    }
    ctx.drawImage(img, sx, sy, sw, sh, 0, 0, cw, ch);
    // canvas cannot encode gif; jpeg/webp pass through, everything else -> png
    const outType =
      file.type === 'image/jpeg' || file.type === 'image/webp' ? file.type : 'image/png';
    const ext = outType === 'image/jpeg' ? 'jpg' : outType === 'image/webp' ? 'webp' : 'png';
    canvas.toBlob(
      (blob) => {
        busy = false;
        if (blob) onApply(blob, ext);
      },
      outType,
      0.92,
    );
  }

  // escape / outside-click flip Bits' open to false; the parent unmounts us on close
  function onOpenChange(open: boolean) {
    if (!open) onCancel();
  }
</script>

<Dialog.Root open {onOpenChange}>
  <Dialog.Overlay class="dlg-scrim" />
  <Dialog.Content class="crop-dlg panel">
    <Dialog.Title level={3} class="crop-h">{title}</Dialog.Title>
    <p class="hint muted">{t('crop.hint')}</p>
    {#if loadErr}<p class="loaderr">{t('crop.failed')}</p>{/if}

    <div
      class="frame"
      style="width:{FW}px;height:{FH}px"
      onpointerdown={onDown}
      onpointermove={onMove}
      onpointerup={onUp}
      onpointercancel={onUp}
      role="presentation"
    >
      {#if url}
        <!-- svelte-ignore a11y_missing_attribute -->
        <img
          bind:this={img}
          src={url}
          onload={onImgLoad}
          onerror={() => (loadErr = true)}
          class="src"
          class:ready
          style="transform: translate({tx}px, {ty}px) scale({scale}); transform-origin: 0 0;"
          draggable="false"
        />
      {/if}
    </div>

    <label class="zoom">
      <span>{t('crop.zoom')}</span>
      <input
        type="range"
        min="1"
        max="4"
        step="0.01"
        value={zoom}
        disabled={!ready}
        oninput={(e) => onZoom(parseFloat((e.currentTarget as HTMLInputElement).value))}
      />
    </label>

    <div class="row">
      <div class="spacer"></div>
      <button type="button" onclick={onCancel}>{t('dialog.cancel')}</button>
      <button class="primary" type="button" onclick={apply} disabled={!ready || busy}>
        {t('crop.apply')}
      </button>
    </div>
  </Dialog.Content>
</Dialog.Root>

<style>
  /* Panel + title ride on Bits components -> global, uniquely named to dodge the
     DialogHost .dlg/.overlay globals. Backdrop is the shared .dlg-scrim. */
  :global(.crop-dlg) {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 61;
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 92vw;
  }
  :global(.crop-h) {
    font-size: 14px;
  }
  .hint {
    font-size: 12px;
    margin: 0;
  }
  .loaderr {
    font-size: 12px;
    margin: 0;
    color: var(--danger);
  }
  .frame {
    position: relative;
    overflow: hidden;
    background: var(--bg);
    border: 1px solid var(--seam-bright);
    border-radius: var(--radius-sm);
    cursor: grab;
    touch-action: none;
    user-select: none;
    align-self: center;
  }
  .frame:active {
    cursor: grabbing;
  }
  .src {
    position: absolute;
    top: 0;
    left: 0;
    max-width: none;
    opacity: 0;
    will-change: transform;
  }
  .src.ready {
    opacity: 1;
  }
  .zoom {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: 12px;
    color: var(--fg-dim);
  }
  .zoom input {
    flex: 1;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .spacer {
    flex: 1;
  }
</style>
