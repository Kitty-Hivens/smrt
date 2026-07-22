// Motion primitives shared by the panel.
//
// The visual language here is flat and mechanical -- borders rather than
// elevation, contrast rather than colour -- so movement follows: short, linear
// -out easings, no overshoot, no bounce. Anything that springs would read as a
// different product. Durations and easings live in app.css as tokens; these are
// the behaviours that need JavaScript.

/// Requests currently in flight, for the shell's activity rail. A counter
/// rather than a boolean: overlapping requests must not have the first one to
/// finish declare the app idle.
let inflight = $state(0);

export const activity = {
  get busy(): boolean {
    return inflight > 0;
  },
  begin() {
    inflight++;
  },
  end() {
    inflight = Math.max(0, inflight - 1);
  },
};

function reduced(): boolean {
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true;
}

/// Reveal a list in sequence rather than all at once, so the eye follows the
/// order the rows arrive in. The per-row delay is capped by index: a 97-mod
/// pack must not take four seconds to appear, so the stagger runs out after the
/// first dozen and the rest land together.
export function stagger(node: HTMLElement, index: number) {
  const apply = (i: number) => {
    node.style.setProperty('--stagger', reduced() ? '0ms' : `${Math.min(i, 12) * 16}ms`);
  };
  apply(index);
  return { update: apply };
}

/// Count a number up to its value on first paint. Used only on the overview
/// tiles, where the number IS the content -- everywhere else a counting digit
/// would be decoration pretending to be information.
export function countUp(node: HTMLElement, value: number) {
  let raf = 0;
  const DURATION = 420;

  function run(to: number) {
    cancelAnimationFrame(raf);
    if (reduced() || to === 0) {
      node.textContent = String(to);
      return;
    }
    const from = Number(node.textContent?.replace(/\D/g, '') ?? 0) || 0;
    const t0 = performance.now();
    const tick = (now: number) => {
      const p = Math.min(1, (now - t0) / DURATION);
      // the same -out curve the CSS tokens use, so a counting number and a
      // sliding panel feel like one system
      const eased = 1 - Math.pow(1 - p, 3);
      node.textContent = String(Math.round(from + (to - from) * eased));
      if (p < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
  }

  run(value);
  return {
    update: run,
    destroy: () => cancelAnimationFrame(raf),
  };
}
