// Which Java a pack runs on (#126).
//
// The field was typed by hand, which is how a pack ends up claiming a Java it
// cannot start under. The set is small and closed, so it is a list -- inventing
// an endpoint to serve six numbers would be sillier than typing them.
//
// The suggestion is a suggestion. The field still takes whatever is chosen: an
// archival pack pinned to an old toolchain is a real thing, and a default that
// argued with the operator would be worse than the free text it replaces.

/// Every Java the mirror expects to see in a pack. Includes the ones only old
/// packs use: 11 and 16 are nobody's choice today and are exactly what an
/// archival pack needs to be able to say.
export const JAVA_MAJORS = [8, 11, 16, 17, 21, 25] as const;

/// Loaders whose whole purpose is to run an old Minecraft on a new Java, so the
/// Minecraft version says nothing about the answer.
///
/// This is not a guess: the mirror's own 1.7.10 pack runs Java 21 through
/// lwjgl3ify, and deriving from the Minecraft version alone would confidently
/// tell its curator 8.
const MODERNISING_LOADERS: Record<string, number> = {
  lwjgl3ify: 21,
  cleanroom: 21,
};

/// Vanilla's own requirement, by Minecraft version. Compared piecewise rather
/// than as a string: "1.9" is not older than "1.12" lexically.
const VANILLA: { from: [number, number, number]; java: number }[] = [
  { from: [1, 20, 5], java: 21 },
  { from: [1, 18, 0], java: 17 },
  { from: [1, 17, 0], java: 16 },
  { from: [0, 0, 0], java: 8 },
];

function parts(version: string): [number, number, number] {
  const n = version
    .split('.')
    .map((p) => Number.parseInt(p, 10))
    .map((v) => (Number.isFinite(v) ? v : 0));
  return [n[0] ?? 0, n[1] ?? 0, n[2] ?? 0];
}

function atLeast(a: [number, number, number], b: [number, number, number]): boolean {
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] > b[i];
  }
  return true;
}

/// What this pack most likely needs. `null` when nothing sensible can be said --
/// an unparseable Minecraft version is not an argument for a number.
export function suggestedJava(minecraftVersion: string, loader: string): number | null {
  const modernising = MODERNISING_LOADERS[loader.trim().toLowerCase()];
  if (modernising) return modernising;
  const mc = minecraftVersion.trim();
  if (!/^\d+(\.\d+)*$/.test(mc)) return null;
  const v = parts(mc);
  return VANILLA.find((rule) => atLeast(v, rule.from))?.java ?? null;
}
