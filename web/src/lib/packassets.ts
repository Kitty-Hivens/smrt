// Where a pack's own files live in its static tree.
//
// They lived under `_nexira/`, which is the name of one launcher -- the
// reference deployment's, at that. Every pack on every self-hosted mirror got a
// directory named after somebody else's client, in the storage layout rather
// than in a string somebody reads. The mirror is meant to be pointed at anyone's
// packs, and the README says so.
//
// The leading underscore is the part worth keeping: the static tree maps into a
// Minecraft instance, so a pack's own files need a name no game directory will
// ever take.
//
// Forward-only, and no migration. A stored path is just a path: `_nexira/...`
// keeps resolving for every pack that already has one, and nothing has to be
// rewritten for the old and the new to sit side by side. Only what gets minted
// from here on changes.

/** Where new pack files are written. */
export const ASSET_PREFIX = '_pack';

/**
 * Prefixes a pack may already be using. Read, never written -- the sweep that
 * keeps an icon resolving to exactly one file has to see the old name, or a
 * re-upload leaves the previous image behind under it.
 */
export const LEGACY_ASSET_PREFIXES = ['_nexira'];

/** Every prefix a pack's own files may sit under, newest first. */
export const ASSET_PREFIXES = [ASSET_PREFIX, ...LEGACY_ASSET_PREFIXES];

/** `_pack/icon.png` -- the path a freshly uploaded pack file gets. */
export function assetPath(...segments: string[]): string {
  return [ASSET_PREFIX, ...segments].join('/');
}

/**
 * Whether `path` is this pack file under any prefix the pack may be using --
 * `_pack/icon.` and `_nexira/icon.` both answer to `icon`.
 */
export function isPackFile(path: string, stem: string): boolean {
  return ASSET_PREFIXES.some((p) => path.startsWith(`${p}/${stem}.`));
}
