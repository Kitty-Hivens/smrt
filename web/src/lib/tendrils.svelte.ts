// Where the tendrils are, so one canvas can paint all of them.
//
// Svelte Flow gives every edge its own <g> and its own SVG paths. That is fine
// for a handful and hopeless for a pack: five hundred paths whose geometry
// changes every frame means a thousand DOM writes and a full re-raster per frame,
// and it measured at six frames a second. The wall is the element count, not the
// maths -- cutting vertices does not move it.
//
// So the library keeps doing what it is good at (laying the graph out, deciding
// where each edge starts and ends, hit-testing a click) and the painting moves to
// a single canvas. Each edge registers its geometry here when it changes -- not
// per frame -- and the layer reads the lot once a frame and draws.
//
// Deliberately not `$state`: the canvas redraws every frame from its own loop, so
// reactivity here would only cost invalidations nobody reads.

export type Tendril = {
  /** endpoints in flow coordinates; the layer applies the viewport transform */
  sx: number;
  sy: number;
  tx: number;
  ty: number;
  /** relation kind -- the layer resolves it to a colour from the panel's tokens */
  kind: string;
  /** per-edge offset so the whole graph does not pulse in lockstep */
  phase: number;
  /** node ids, so the layer can tell what the hovered mod's path is */
  source: string;
  target: string;
};

const items = new Map<string, Tendril>();

export const tendrils = {
  set(id: string, t: Tendril): void {
    items.set(id, t);
  },
  remove(id: string): void {
    items.delete(id);
  },
  all(): Map<string, Tendril> {
    return items;
  },
};
