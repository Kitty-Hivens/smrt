<script lang="ts">
  import { useSvelteFlow } from '@xyflow/svelte';

  // Re-frames the camera whenever the graph is rebuilt. `fitView` on <SvelteFlow>
  // only runs at mount, so refocusing (which re-lays-out a whole new set of nodes)
  // would otherwise leave the camera parked over where the old layout used to be
  // and the view looking empty. Lives inside <SvelteFlow> because that is where
  // the flow's context -- and so `useSvelteFlow` -- is available.
  let { token }: { token: unknown } = $props();

  const { fitView } = useSvelteFlow();

  $effect(() => {
    token; // re-fit on every new layout
    // next frame, so the fresh nodes are measured before the camera is framed
    const raf = requestAnimationFrame(() => {
      void fitView({ padding: 0.2, duration: 300 });
    });
    return () => cancelAnimationFrame(raf);
  });
</script>
