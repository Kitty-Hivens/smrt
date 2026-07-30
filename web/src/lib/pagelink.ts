// Where the next page of a listing lives.
//
// The mirror pages by cursor and names the next page in a `Link` header rather
// than wrapping every listing in an envelope, so the body a paging caller reads
// is the same body an unpaged one reads. Reading the address back out is the
// one piece of that worth keeping honest on its own: it is the difference
// between a walk that terminates and one that silently stops at page one.

/// The `rel="next"` address from a `Link` header, or null when the listing has
/// no more pages. Only the one relation the mirror emits is understood; a header
/// naming others is not an error, it simply has no next in it.
export function nextPageUrl(link: string | null | undefined): string | null {
  if (!link) return null;
  for (const part of link.split(',')) {
    const match = part.match(/^\s*<([^>]+)>\s*;\s*rel\s*=\s*"?next"?\s*$/);
    if (match) return match[1];
  }
  return null;
}
