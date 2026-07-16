<script lang="ts">
  import type { EdgeProps } from '@xyflow/svelte';
  import { tendrils } from '../lib/tendrils.svelte';

  // A living relation line. The drawing itself happens on one canvas
  // (`TendrilLayer`) rather than here: five hundred SVG paths whose geometry
  // changes every frame is a thousand DOM writes per frame, which measured at six
  // frames a second on a real pack. So this component does the two things the
  // library is genuinely better at -- knowing where the edge runs, and catching a
  // click on it -- and hands the geometry to the layer to paint.
  //
  // What is left in the DOM is one straight, invisible, never-animated stroke: the
  // hit area, so a debug user can still select an edge and delete it.
  let { id, source, target, sourceX, sourceY, targetX, targetY, data }: EdgeProps = $props();

  const kind = $derived((data?.kind as string) ?? 'requires');
  // per-edge offset so the whole graph does not pulse in lockstep
  const phase = $derived((data?.phase as number) ?? 0);

  // register on change -- not per frame; the layer reads the lot once a frame
  $effect(() => {
    tendrils.set(id, {
      sx: sourceX,
      sy: sourceY,
      tx: targetX,
      ty: targetY,
      kind,
      phase,
      source,
      target,
    });
    return () => tendrils.remove(id);
  });
</script>

<path class="hit" d="M{sourceX},{sourceY} L{targetX},{targetY}" />

<style>
  .hit {
    fill: none;
    stroke: transparent;
    stroke-width: 14;
    stroke-linecap: round;
    pointer-events: stroke;
  }
</style>
