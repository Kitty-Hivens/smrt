// Transient graph-interaction state, shared straight with the canvas that paints
// the edges. It lives outside the edge array on purpose: touching that array would
// re-run the layout and jump the whole graph. Each tendril reads these and decides
// for itself, so an interaction costs one reactive read per edge and no relayout.

// Which node the pointer is on (lights its path, dims the rest).
let id = $state<string | null>(null);

export const hover = {
  get id(): string | null {
    return id;
  },
  set(nodeId: string | null) {
    id = nodeId;
  },
};

// True while a node is being dragged. Dragging re-renders the nodes every pointer
// move -- that is the library's own heavy work -- so the canvas stops animating the
// wave and draws the tendrils flat for the duration, following the node without
// churning a sine per vertex on top of the drag. The wave resumes on drop.
let dragging = $state(false);

export const drag = {
  get active(): boolean {
    return dragging;
  },
  set(on: boolean) {
    dragging = on;
  },
};
