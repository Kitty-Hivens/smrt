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

- The mirror stands alone as a self-hostable product: deployment-specific
  values (operator uid, public base URL) moved to the environment; the
  SmartyCraft/Nexira setup is the reference deployment, not the definition.

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
