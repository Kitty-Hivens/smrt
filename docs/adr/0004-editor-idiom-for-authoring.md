# 0004. Authoring surfaces are an editor, not a form

Status: accepted, not yet built

## Context

PackEditor is literally an editor: you manipulate a pack. It is built as a set of
CRUD forms and modal dialogs. The dependency graph, by contrast, was built as a
real editor: direct manipulation, an inspector-like focus panel, hover reveal,
role-gated drag-to-author-edges. The graph proves the project can do rich
interaction. The care landed on the graph and skipped the core authoring loop.

## Decision

Build the authoring surfaces (PackEditor, sources, the pack's own graph) in the
editor idiom, not as forms. Concretely, an action like "change version" is a
glyph in the row that opens a right-side inspector (versions, channel, date,
compatibility); the value settles into place with motion; the row updates. The
pieces already exist (the version data, the picker logic in ModrinthPicker step
2). The work is rearranging them from modal-over-everything into an inspector in
context.

A layered window must not blend into the surface it covers. Four rules:

- It is a distinct surface, not the same tone: a clearly higher elevation level,
  or its own tint. Which of the two is still open.
- The content behind it is dimmed by a scrim (`.dlg-scrim` exists).
- It carries an edge: a border or `--shadow-pop`.
- It animates in through `--ease-out` so its arrival as a layer is legible. The
  overshoot allowed by 0003 is for small controls, not for a large panel.

## Rejected

- The editor idiom everywhere. Consumption surfaces (the public catalog) do not
  want inspectors; simpler is better there. This idiom is for surfaces where you
  create.

## Consequences

- Depends on 0003. Colour, depth, and motion were all banned, and a big inspector
  needs all three to read as a window.
- This is the concrete form of "a UI that leads": the next step is a glyph in
  context, not a guess about which section to switch to.
- The graph stays as it is, an enthusiast tool with a high floor. A newcomer gets
  a separate, simpler surface, not a softened graph.
