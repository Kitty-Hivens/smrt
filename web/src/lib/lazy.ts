// Loading a part of the panel that arrives on demand.
//
// The panel's chunks are named after a hash of their contents and are shipped
// inside the binary, so a deploy replaces the whole set at once: the names an
// already-open page knows stop existing the moment the mirror restarts. The page
// itself keeps working until it reaches for one of them -- the registry and the
// graph are loaded on first use -- and then the import fails on a file that is
// simply gone.
//
// The page cannot be patched, but it can be replaced: reloading fetches the
// current shell, which names chunks that exist. Once only, so a chunk that is
// genuinely missing surfaces as an error instead of looping.

const RETRIED = 'smrt.chunk-retry';

function remember(v: string | null) {
  try {
    if (v === null) sessionStorage.removeItem(RETRIED);
    else sessionStorage.setItem(RETRIED, v);
  } catch {
    // blocked storage: the retry simply does not survive the reload, which
    // costs one extra attempt at worst
  }
}

function retried(): boolean {
  try {
    return sessionStorage.getItem(RETRIED) === '1';
  } catch {
    return false;
  }
}

/// Import a chunk, surviving a deploy that happened while this page was open.
export function lazy<T>(load: () => Promise<T>): Promise<T> {
  return load().then(
    (mod) => {
      remember(null); // this page's chunks resolve; a later failure is a fresh one
      return mod;
    },
    (err) => {
      if (retried()) throw err;
      remember('1');
      location.reload();
      // The page is on its way out; never settling keeps the caller in its
      // loading state rather than flashing an error nobody has time to read.
      return new Promise<T>(() => {});
    },
  );
}
