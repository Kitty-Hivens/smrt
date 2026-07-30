// Whether a server for a pack will tell you its mod list before you connect
// (#148).
//
// The handshake claim (#110) is copied from what a server advertises in its
// status ping. That only works where the ping carries a mod list, and which
// loaders do is not something the panel should leave to the request: a pack's
// loader is known before anyone presses anything, and pressing a button that
// cannot work and reading why afterwards is a worse answer than saying so first.
//
// The three eras:
//
//   FML1 (Forge, 1.12.2 and earlier)  `modinfo.modList` -- [{modid, version}]
//   FML2/3 (Forge, 1.13 onward)       `forgeData` -- a compressed mod list
//   NeoForge, Fabric, Quilt           nothing
//
// The Minecraft version does not enter into it. Forge advertised in every era,
// under two different spellings, and the forks that inherit a Forge already say
// which era they are by being that fork.
//
// NeoForge dropped it because the negotiation moved: it happens after connecting,
// in the configuration phase, over registered channels rather than a declared
// list. That is also why the claim does not port there -- a channel is a live
// pipe, not an assertion, and presenting one you do not implement gets past
// negotiation and then dies on the first packet sent over it.

/** Loaders that are a Forge of some era, including forks that inherit one. */
const FORGE_FAMILY = new Set(['forge', 'cleanroom', 'lwjgl3ify']);

/**
 * Whether a server for a pack on this loader advertises a mod list in its status
 * ping -- which is to say, whether a handshake claim can be derived at all.
 */
export function advertisesModList(loader: string): boolean {
  return FORGE_FAMILY.has(loader.trim().toLowerCase());
}
