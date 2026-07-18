# Concepts

The domain model, in the order a curator meets it.

## Pack, config, manifest

A **pack** is one curated modpack, identified by a stable string id (`Industrial`,
`Create`). Its **config** (`authoring/config.json`) is the curator's
declaration: every mod and asset, each with exactly one source and an
install-time default (`default_enabled`). Its **manifests** are frozen builds:
the config resolved into exact artifacts (sha1 + size + URL per entry) that a
launcher can reproduce byte for byte.

There is no hand-set "required" flag anywhere in a config. Required-ness is
derived at build time (below); the curator's only lever is `default_enabled`.

## Versions and channels

The version model follows Modrinth: **the version string is a plain number and
carries no channel semantics; the channel is a separate stored field.**

- **Version number**: `<MAJOR.MINOR base>.<counter>`. The base is the
  hand-bumped `version` line in the config; the counter is assigned per build,
  one past the highest already published for that base, starting at zero
  (`0.1.0`, `0.1.1`, ... -> bump the base to `0.2` -> `0.2.0`). A deleted
  build's number is never reissued. Historical labels (date-based releases,
  `SNAPSHOT-...` panel builds) remain valid for already-published manifests.
- **Channel**: `release | beta | alpha` -- the Modrinth `version_type`
  vocabulary, stored on the manifest (`channel`), chosen at build time (panel
  selector, `smrt-pack build --channel`, `?channel=` on the build endpoint).
  The default is **beta**: publishing a release is an explicit act. Manifests
  from before the field are read through a legacy fallback (`SNAPSHOT-` prefix
  = beta, anything else = release).
- **Ordering**: numeric tuple comparison within a base (`0.4.10` after
  `0.4.2`; string sort is wrong). Across bases or eras, order by the build
  timestamp (`generated_at` / `date_published`), which every build carries.

The registry's mod releases use the same channel vocabulary, so "a beta" means
one thing everywhere.

## Sources

Every mod/asset entry names exactly one source:

- **`modrinth`** (`project_id` + `version_id`) -- resolved against Modrinth at
  build time; the launcher downloads from Modrinth's CDN and verifies the
  manifest's sha1.
- **`smrt_cache`** (`sha1`) -- a jar the mirror hosts itself,
  content-addressed. Reserved for genuine archival cases: jars Modrinth does
  not carry for that mod x MC x loader. Anything Modrinth carries should be a
  `modrinth` source (moderation policy, enforced at upload review).
- **`smrt_static`** (`rel_path`) -- a mirror-hosted file under the pack's
  static tree (configs, options, resource packs, UI overrides).

## The mod registry

Modrinth-shaped identity over everything the mirror has seen:

```
mods (identity: name, slug, author, env flags)
  -> mod_release (version_number + channel)
       -> mod_version (one file: sha1, size, filename, loaders, mc versions)
mod_alias    (modid / modrinth project id -> mod)
relation     (dependency edges, by source: authored/curator/jar-meta/modrinth/inferred)
jar_class    (per-sha1: kind + side + match policy + confidence)
```

Rows carry a **source** (`harvested`, `jar-meta`, `modrinth`, `inferred`,
`curator`, `authored`). Harvested layers are wiped and re-derived every
harvest; `curator`/`authored` rows are precious and never overwritten by a
re-harvest. Identity for non-Modrinth jars comes from jar metadata
(`mcmod.info`, `mods.toml`/`neoforge.mods.toml`, `fabric.mod.json` --
including displayName, version with `${file.jarVersion}` resolution, logo,
target MC, and the loader distinction forge vs neoforge); Modrinth re-uploads
are linked by sha1 lookup, slug==modid folds, and a one-time modid read of the
re-uploaded jar.

## Side, match policy, presence

Three orthogonal axes decide how an entry behaves in an install:

- **Side**: `client | server | both` -- where the mod does its work.
- **Match policy**: `must_match | tolerant` -- whether client and server must
  carry it in lockstep (content mods) or not (QoL, libraries).
- **Presence** (wire, advisory): `required | optional_client |
  optional_server | optional_both | coremod` -- the chip a launcher renders;
  absent = unclassified.

Classification runs through one decision layer (`classify_artifact`) with a
strict source cascade:

1. a non-mod jar kind (coremod/library) short-circuits -- never required,
   always toggleable;
2. Modrinth project environment flags (`client_side`/`server_side`) -- the
   priority-1 source; declared metadata often lies, upstream flags rarely do;
3. bytecode-derived verdict (class-level side markers, surface heuristics)
   with a confidence grade -- a low-confidence client verdict yields to a
   declared dependency edge instead of blocking a build.

## Required derivation and the client invariant

At build time:

```
required = { default-enabled must_match mods }            (the seeds)
         + transitive hard deps of every default-enabled mod
```

with the side rules applied on top: server-side mods are never required and
ship opted out; non-mod jars are never required; **a confidently client-side
mod is never required, period** -- a hard edge into one does not lock it
(client chains co-toggle in the launcher via the `requires` tree), and a
classification that would force one fails the build rather than shipping a
manifest that force-installs a client mod on a server. An opted-out
`must_match` mod stays out (the curator's opt-out wins) and still reads
`optional_both`.

## The dependency graph

Edges come from, in order of confidence: authored/curator declarations,
loader manifests (`mods.toml` typed dependencies -- these also suppress
bytecode inference for the same jar), Modrinth version dependencies, legacy
`mcmod.info`, and bytecode inference (package references, graded and
downgraded when they point across sides). `display.requires` on the wire is
the resolved per-pack subset; launchers use it for co-toggling and dependency
trees, the build uses it for the required walk.

**Depfill** (on config save, or `smrt-pack depfill`) walks the declared mods'
hard dependencies and pulls the missing ones into the config -- from Modrinth
first, the mirror's own cache second, honoring the pack's loader family and MC
version and the requirer's version window. `Recommends` edges become curator
suggestions, never auto-adds. Failures are non-fatal: an upstream outage
leaves the config as it was, and the resolve report flags what is still
missing.

## Fingerprint

Every manifest carries a content fingerprint: a sha1 over exactly what lands
in an instance (artifact hashes + install flags + loader/java/MC), independent
of the version label and timestamp. Two builds with identical content share a
fingerprint -- the reliable "did anything actually change?" signal for
launchers and diff tooling.
