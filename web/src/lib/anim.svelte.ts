// One animation clock for the whole view: a single rAF drives every animated
// edge, rather than each edge spinning up its own loop. Consumers read `clock.t`
// inside a $derived and re-render on tick.
//
// `t` counts in 20-per-second "ticks". The unit is not arbitrary: the tendril
// wiggle constants are ported from a tick-driven renderer, so keeping the same
// time base keeps the motion at its intended speed.
//
// Frozen at 0 under prefers-reduced-motion -- the wave keeps its shape, it just
// stops travelling, so the graph stays readable without moving.

let t = $state(0);
let subscribers = 0;
let raf: number | null = null;
let start = 0;

function reducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true
  );
}

function step(now: number) {
  t = ((now - start) / 1000) * 20;
  raf = requestAnimationFrame(step);
}

/**
 * Subscribe to the clock for the caller's lifetime. Call inside `$effect` and
 * return the result, so the clock stops when the last consumer unmounts:
 *
 *   $effect(() => useClock());
 */
export function useClock(): () => void {
  subscribers++;
  if (subscribers === 1 && raf === null && typeof window !== 'undefined' && !reducedMotion()) {
    start = performance.now();
    raf = requestAnimationFrame(step);
  }
  return () => {
    subscribers--;
    if (subscribers === 0 && raf !== null) {
      cancelAnimationFrame(raf);
      raf = null;
    }
  };
}

export const clock = {
  get t(): number {
    return t;
  },
};
