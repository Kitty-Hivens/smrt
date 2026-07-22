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

### Fixed

- A Modrinth version upstream published without a jar is no longer
  selectable: the picker greys it out, auto-fill skips it, and the build
  error says what happened.
- Derived state no longer depends on upstream weather: pulled dependencies
  are sticky across saves and outages, one unresolvable target does not
  abort the fill pass, a degraded Modrinth leg does not wipe harvested
  relations, and builds wait for an in-flight harvest before classifying.
- Modrinth client resilience: hard per-request deadlines and an unfiltered
  fallback for the filtered version listing.
