// Changing everything at once, without the blink.
//
// A theme flips every token and a locale rewrites the text of the whole panel.
// Both are one frame today: the old state is simply gone. A CSS transition is
// the wrong tool -- putting one on colour and background everywhere animates
// every element on every repaint, which turns a theme switch into a smear -- so
// this uses a view transition instead: one snapshot before, one after, and a
// crossfade between them, at no per-element cost.
//
// Deliberately not a loading state. Nothing is loading: the dictionary is
// already in memory and the tokens are already in the stylesheet. A placeholder
// claiming otherwise would buy a moment of calm by spending the credibility of
// every real loading indicator in the product.

/// The one switch. A JavaScript-driven transition asks the same question the CSS
/// tokens answer, so `prefers-reduced-motion` keeps disarming the whole product
/// rather than growing a second control beside it.
function reduced(): boolean {
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true;
}

/// Apply `change`, crossfading the whole page across it where the browser can.
///
/// `name` reaches CSS as `data-transition` on the root, so a change that should
/// read slowly (the text of the panel being rewritten) and one that should read
/// quickly (a substrate swap) can differ without either knowing about the other.
export function withTransition(name: string, change: () => void): void {
  const root = document.documentElement;
  const start = (
    document as Document & {
      startViewTransition?: (cb: () => void) => { finished: Promise<void> };
    }
  ).startViewTransition;

  if (reduced() || typeof start !== 'function') {
    change();
    return;
  }
  root.dataset.transition = name;
  const transition = start.call(document, change);
  void transition.finished.finally(() => delete root.dataset.transition);
}
