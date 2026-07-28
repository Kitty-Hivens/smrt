# Changelog

Notable changes to the smrt mirror. The service deploys continuously from
`main`; entries land under Unreleased as they ship and collapse into a
version section when a release is tagged.

## Unreleased

### Added

- Modrinth-shaped version model for packs: plain `base.counter` version
  numbers, a stored release/beta/alpha channel chosen at build time
  (default beta), and a versions listing speaking Modrinth field names
  (`version_number`, `version_type`, `date_published`, fingerprint, counts).
- Structured build diff for update dialogs:
  `GET /v1/packs/{id}/diff?from=&to=` -- loader/minecraft/java bumps, mods
  added/removed/updated/toggled with registry-enriched version labels.
- Hash-first artifact lookup `GET /v1/files/{sha1}`; mod pages resolve by
  slug and expose the project environment flags.
- Job snapshots: build job ids survive service restarts; a job killed by a
  restart reads failed with an explicit interrupted line.
- Full OpenAPI coverage of the public surface at `/docs`, and a real
  documentation set under `docs/` (architecture, concepts, API guide,
  operations, development).
- Side/required/presence model: per-jar classification (Modrinth env flags
  first, bytecode second) drives derived required-ness with a hard
  invariant -- a client-side mod is never force-installed. Presence classes
  ride the manifest display block.
- Dependency auto-fill on config save: missing hard dependencies pull in
  from Modrinth or the mirror cache; resolved requires graphs feed the
  launcher's dependency tree.
- Modern jar metadata extraction: displayName, version (including
  `${file.jarVersion}` resolution), logoFile and target MC from
  mods.toml / neoforge.mods.toml / fabric.mod.json; NeoForge jars register
  under the `neoforge` loader; jar-embedded icons serve for modern mods.

### Changed

- A view reflows against its own column, not against the window. Every
  responsive rule in the panel asked the viewport, which is the same question
  only while a view is the whole screen -- so a narrow window and a narrow
  column disagreed about identical content, and a view could never be one pane
  beside another. The content area is a named container now and the view-level
  rules ask it; the shell's own layout (the rail becoming a strip, then a
  drawer) stays window-keyed, because that is a fact about the screen. Measured:
  at a 560px column inside a 1400px window the editor's basics grid collapses to
  one column and the mod row becomes a card, which a viewport rule cannot see.
  The report dock follows the same box -- a container is the containing block
  for the fixed panels inside it, so the dock now belongs to its view instead of
  floating over the rail, and its drag bounds measure the column.

### Changed

- The mirror stands alone as a self-hostable product: deployment-specific
  values (operator uid, public base URL) moved to the environment; the
  SmartyCraft/Nexira setup is the reference deployment, not the definition.

### Added

- A settings surface, and a light theme to put in it. The panel had nowhere for
  a preference to live -- the locale switch sat in the top bar because there was
  no other place -- and the theme could not exist at all: the tokens were
  dark-first with white and black tints written literally at their use sites, so
  on a light background the seams, the dot field, the table zebra and the scrims
  inverted. Those are values now, and the light half is measured rather than
  guessed: every text tier clears its floor against the field it sits on (the
  binding case on paper, not the card), and the four status hues are re-solved
  because the dark greens and ambers land near 1.6:1 there. Elevation swaps with
  the substrate -- a shadow on paper, a lighter surface on black -- which is what
  the token file already said and could not do. The paper is deliberately not
  office-white: the first pass took the surfaces to 0.97 luminance and glared,
  since on an emissive screen the brightest thing in the room should not be the
  empty half of a form. Follow-the-system is the default
  and keeps following as the desktop changes; the choice is applied before first
  paint, so a light session never flashes black. The launcher preview keeps its
  own dark, since it renders another product.
- A pack still builds while Modrinth is down. Every Modrinth-sourced mod
  resolved its hash and size live from Modrinth at build time, so a pack with
  one such mod could not be rebuilt until upstream came back -- on exactly the
  class of mod the mirror deliberately keeps no bytes for. Harvest had already
  recorded those numbers, and the build falls back to them: the network is asked
  first and stays authoritative, so a re-uploaded version is never built from
  stale numbers, but a version the registry knows no longer takes the build down
  with it. A pin the mirror has never harvested still fails, naming both the
  upstream failure and the missing local record. The build log says when it
  answered from the registry -- a build that reached upstream and one that did
  not are different events.
- Adding a mod says what comes with it. The dependencies used to arrive later,
  on save, so whoever added a mod found out afterwards -- by opening the preview
  or resolving by hand -- and a pack quietly gained jars nobody chose to look
  at. The plan was always computed; it is now asked for at the moment of the add
  (`POST /v1/authoring/packs/{id}/dependency-preview`, read-only: it runs the
  real fill on a copy so the answer cannot drift from what the save does, and
  writes nothing) and reported as one line naming what is coming. Silent when a
  mod brings nothing, which is most of them. The rows themselves now appear as
  soon as the save returns, too: the mirror always answered with the config it
  stored, dependencies included, and the editor was discarding that answer.

### Added

- The panel's state lives in the URL. Sections are paths, a mod page is a
  shareable link, and the server serves the app shell for any path it does
  not claim -- so back and forward (and the mouse buttons wired to them) work,
  a reload keeps the mod page you had open, and a link to what you are
  looking at can be sent to someone. Navigation was a variable and a
  localStorage key before, which the browser knew nothing about.
- A motion system, where the panel had fifteen hand-picked CSS durations and
  nothing else: three duration tokens and two easings, short and linear-out to
  match a flat interface, with one `prefers-reduced-motion` rule that disarms
  the whole product by zeroing them. Requests in flight show as a hairline
  wire under the top bar -- one place for "work is happening" instead of a
  spinner per view; long lists reveal in sequence rather than as a block; the
  dock arrives as a panel being placed; controls take a press; the overview
  counts its numbers up once.

### Fixed

- The panel and every curated pack asset are served again. The axum 0.8 route
  migration wrote the two wildcard routes with escaped braces (`{{*path}}`),
  which matches a literal path rather than any real request: every panel asset
  was answered with the app shell, so the panel loaded as a blank page, and
  every `/v1/packs/{id}/static/...` file -- pack icons, banners, and any
  `smrt_static` source a manifest points at -- answered 404. Both routes are
  pinned by tests that assert the request reaches its handler.
- Navigation is links. Every place in the panel had an address -- sections are
  paths, a mod page and now a pack editor are their own URLs -- but nothing
  wore them: eighteen navigations were click handlers on buttons and rows
  against two `<a href>` in the whole product. So the middle click did nothing,
  ctrl-click did nothing, "copy link" copied the page you happened to load
  first, and a screen reader announced "button" for a destination. The rail,
  pack rows, mod names and the graph's open-page control are links now, with
  the handler intercepting a plain click for client-side routing and leaving a
  modified one to the browser.
- Form fields say what they are. The visible caption a `Field` draws was a
  plain span with nothing tying it to the control, so a screen reader on the
  input heard silence -- and where an `aria-label` existed it repeated the
  placeholder, naming the example ("main", "https://...") instead of the field
  and overriding the caption that was right there. The caption is now named and
  the control points at it, in the one component every labelled field goes
  through; the example-echoing labels are gone, and controls too dense for a
  caption (mod rows, the asset table, the drop zones) carry a real field name
  instead. Measured on a running panel: the pack editor and the server editor
  have no unnamed control and none named by its own example.
- The pack editor can be left with the back button. It opened from local state
  while the URL stayed on the section, so nothing about opening it entered
  history: back walked out of the editor's own view or out of the panel
  entirely, and a reload lost the pack you had open. The open editor is a
  location now (`/packs/<id>`, `/mypacks/<id>`), so back and the trackpad
  gesture close it and a link reopens it. The unsaved-changes guard moved with
  it: it lives on the route rather than on the Close button, so every way out
  asks -- declining walks back to the editor rather than stacking a history
  step.
- Two accounts editing one pack no longer lose each other's work. The config
  read answers with an `ETag` of its authored content and a save may carry it
  as `If-Match`; a save whose base has moved on is refused with 409 instead of
  overwriting whoever saved first, and the pack's whole load-merge-write runs
  under a per-pack lock so two saves cannot interleave either. The editor keeps
  the refused edits on screen, stops autosaving, and asks which version wins --
  take the stored one, or save over it, the latter rebasing onto the current
  revision so it stays a conditional write. Server-side changes a client cannot
  cause -- publishing a pack, a pulled dependency appearing -- are outside the
  revision, so they never reject an edit in flight, and a request that sends no
  `If-Match` (the CLI, a script) writes unconditionally as before.
- The curator slug is offered where it does something. It is load-bearing for
  a self-hosted mod, whose filename changes under it and which has no project
  id -- so a Modrinth row now states what actually keys it instead of showing
  an empty field that changes nothing, and the source column fits the word
  `modrinth` rather than spending the same width on an ellipsis. The three
  identities a mod has (file, registry, across-builds) are written down in
  `docs/concepts.md`.
- The activity counter no longer turns a one-shot fetch into a request loop.
  Counting in-flight requests through reactive state meant any request started
  inside an effect made that effect depend on its own side effect: the shell's
  single health fetch became an unbounded loop that starved every other
  request on the page, so views rendered empty. Only the derived flag is
  reactive now.
- A failure notice shows the server's sentence, not its envelope: four lines
  of JSON where the actual problem was one line inside it.
- The report dock opens below the view's header instead of on top of the
  controls that opened it.
- Panel-wide design pass. The type scale had eight sizes inside a four-pixel
  band, bottoming out at 9px on the public catalog; it is six steps now, and
  nothing in the product is smaller than an 11px mono label. Every control
  has a 28px minimum target, where the compact variants sat at 24-26px with
  several of them packed side by side in a row. Thirty-seven inputs whose
  only name was a placeholder -- which disappears exactly when you type --
  carry an accessible name.
- Failures no longer move the page anywhere, not just in the pack editor.
  Fourteen views inserted an error banner at the top of their content; they
  all report through the notice stack now. A dialog keeps its own inline
  error: it is already an overlay, and the failure belongs to the thing in
  front of you.
- Palette defects the tokens themselves documented: faint text measured
  3.86:1 on the panel surface it actually sits on (the note claimed 4:1,
  which held only against pure black) and is now 4.51:1; the four soft state
  tints shared one 0.14 alpha and landed at 1.13-1.21:1 -- invisible as fills,
  which is why state read off borders alone -- and now carry per-hue alphas
  solved to equal perceived weight; the retired-for-contrast `--accent-dim`
  and the single-use second red are gone.
- The editor stops moving under the cursor. Reports and failures were
  inserted at the top of the form, so asking for a resolve or hitting an
  error pushed everything down by however tall the answer was. Reports now
  open in a draggable dock that overlays the page and remembers where it was
  parked; failures are notices in a fixed corner stack, with the rejected
  save carrying its reason and a retry. Nothing in the flow reflows.
- The top-bar refresh works on every view. It bumped a shared signal only one
  view listened to, so on the registry, graph, my-packs and public catalog it
  was a button that did nothing -- while the graph kept a second refresh of
  its own beside it. Every view now listens, the duplicate is gone, and the
  button is offered to any signed-in user rather than operators alone.
- The pack editor no longer loses edits quietly. A rejected autosave was a
  grey word in the header with the reason only in a tooltip, and it never
  retried, so a failed save plus a closed tab meant the work was gone. It is
  now a banner with the server's reason, a retry, and a confirmation before
  leaving. Emptying the Java field no longer sends a null into a required
  field, and switching a mod's source type keeps the reference it had, so a
  stray click on the dropdown is recoverable.
- Uniqueness holds on every add path, not just the pickers: dropped jars go
  through the same identity check, and assets are unique by `dest` in the
  editor and on save -- two rows writing one path installed whichever the
  launcher fetched last.
- A loader that ships a Forge mod's capability natively is understood as
  answering that dependency. Cleanroom loads mixins itself, so MixinBooter
  (the Forge backport) is redundant on a Cleanroom pack -- but removing it
  used to leave Entity Culling and Relictium with an unsatisfied mixinbooter
  dependency the resolver flagged as missing. A `loader_provides` seed, keyed
  to the exact loader, now records what the loader covers; the dependency
  resolves clean and auto-fill does not pull the mod back. One row per
  capability, no code change to add another.
- A connector's `loader:<name>` capability is now shipped as data and emitted
  by the harvest, so a Fabric mod carried by Sinytra Connector reads as
  carried instead of "will not load". The resolver understood bridges
  already; nothing ever produced the fact, so on any fresh mirror every
  bridged mod was a false alarm. Add a niche connector with one row in
  `loader_bridge` -- no code change.
- Dependency auto-fill no longer waits for a build: a Modrinth pin the
  harvest has not read yet contributes its dependencies straight from the
  version it declares, so a mod just added to a config -- or re-pinned to a
  newer build -- pulls its libraries immediately instead of after the pack
  has been built and harvested once. A dependency that names an exact
  version is pulled at that version.
- A Modrinth version upstream published without a jar is no longer
  selectable: the picker greys it out, auto-fill skips it, and the build
  error says what happened.
- One row per mod: configs declaring the same artifact twice, or two rows
  writing the same `mods/<filename>`, are refused on save, and the pickers
  no longer offer what the pack already ships. Artifact identity ignores the
  pinned version, so a second version of a mod already in the pack counts as
  a duplicate rather than a new entry.
- Derived state no longer depends on upstream weather: pulled dependencies
  are sticky across saves and outages, one unresolvable target does not
  abort the fill pass, a degraded Modrinth leg does not wipe harvested
  relations, and builds wait for an in-flight harvest before classifying.
- Modrinth client resilience: hard per-request deadlines and an unfiltered
  fallback for the filtered version listing.
