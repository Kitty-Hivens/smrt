// Which graph node the pointer is on, shared straight with the edges.
//
// It lives outside the edge data on purpose. Hovering must not rebuild the edge
// array: that would re-run the layout and jump the whole graph under the cursor.
// Each tendril reads this and decides for itself whether it is on the hovered
// mod's path, so a hover costs one reactive read per edge and no relayout.

let id = $state<string | null>(null);

export const hover = {
  get id(): string | null {
    return id;
  },
  set(nodeId: string | null) {
    id = nodeId;
  },
};
