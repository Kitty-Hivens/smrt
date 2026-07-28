# 0005. A panel is a place, a tool, or an interruption

Status: accepted, not yet built

## Context

The panel has four idioms for putting something above something else, and seven
hand-rolled instances of them: five separate `Dialog.Root` mountings (the
Modrinth, mirror and GitHub pickers, the image cropper, the identity dialog), the
promise-based confirm/prompt/choose host, the draggable `FloatDock` that holds
reports, and the off-canvas rail with its own scrim. Each decides for itself how
it is sized, how it closes, whether it traps focus, and whether anything about it
survives a navigation.

That count is the problem. A fifth idiom would not fix it and neither would one
universal panel, because the things being layered are not the same kind of thing.
A pack editor is somewhere you are. A resolve report is something you hold while
working somewhere. A delete confirmation is neither -- it is a question that must
be answered before anything else continues.

0004 already commits the authoring surfaces to the editor idiom, with an
inspector in context rather than a modal over everything. This record is the
container that idiom needs, and the rule for everything else that layers.

The mirror's work is cross-referential: authoring a pack while reading a mod page
while searching what the mirror holds. Overlays force a choice -- close one to
see another. `FloatDock` was already an admission of this: reports were pulled
out of the flow so the editor underneath would stop reflowing. The dock is the
nucleus, not the casualty.

## Decision

Three kinds, each with its own law.

**A place** is somewhere you are: the pack editor, a mod page, search, the
registry. It lives in the URL, it is reachable by link, back leaves it, and its
data is fetched by the route entry that owns it. Places stack, and the stack is
in the address, so "back" means "drop the top pane" rather than "leave the app".

**A tool** is something you hold while working: a resolve or validate report, the
build console, a graph legend. It is not in the URL -- it answers "what do I have
open", not "where am I". It is keyed by a stable id, remembers its geometry
across sessions and navigations, fetches its own data on mount so opening one
never blocks the page, and **does not trap focus**: editing the form while
reading the report is the entire point.

**An interruption** is a question that blocks: a confirmation, the image cropper,
accepting the rules. No id, no address, no persistence, focus trapped, dismissed
by answering. These stay modal, and they are the only things that do.

One primitive renders all three, so arrival, dismissal, elevation, scrim and
anchoring are decided once. It absorbs the five hand-rolled dialogs and
generalises `FloatDock` rather than replacing it. Opening a panel that is already
open raises it instead of spawning a second copy. A panel opens from the
rectangle of the control that opened it, so the motion says where it came from.
On a narrow container a panel becomes a full-width sheet -- automatically, once
the reflow rules key off the container rather than the viewport.

The context menu is the same primitive anchored to a point. Custom menus are
added only where we have actions the browser cannot offer -- a mod row (open the
mod page, re-pin the version, remove from the pack), a pack row (publish,
duplicate, delete), a registry row (assign identity, take down). On text, inputs
and links the native menu stays: replacing it costs open-in-new-tab, copy, spell
check and the browser's own accessibility path, and buys only a consistent
look -- which is not even consistent, since the native menu is themed by the
system.

## Consequences

The reflow rules must key off the container before any of this is usable. The
panel has 21 `@media` rules and no container queries, so a 520px pane inside a
1920px window would keep rendering the three-column basics grid and the
seven-column mod row. The narrow layouts already exist and are tested -- they
were written for phones -- so this is re-pointing them, not inventing them. It is
also worth doing on its own: today a narrow window and a narrow column disagree
about the same content.

Making everything a place would fill the history with panes, so back becomes
"close another layer" forever. Which surfaces are places is decided one at a
time, and the default is tool.

The URL grows a stack. The route store holds three separate slots today (section,
mod, pack); an ordered stack replaces them, and every entry needs to survive a
reload, which is what makes a pane linkable in the first place.

Navigation had to become links before any of this: the addresses existed but
nothing wore them, so the browser could not open, copy or announce a destination.
That is done.

## Rejected

**One universal panel.** Collapsing the three kinds into one leaves either a
confirmation you can navigate away from or a report that steals focus.

**Keeping the five hand-rolled dialogs.** They are the same widget five times,
each with its own closing rules; the inconsistency is invisible until you use two
of them in one session.

**Replacing the native context menu everywhere.** Consistency of appearance is
not worth the capabilities it removes, in a product served through a browser.
