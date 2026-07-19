# Architecture

smrt is a self-hostable mod mirror and pack registry. It answers one question
end to end: *given a pack id, what exactly lands in a client instance, and
where does every byte come from?* Everything else -- the registry, the panel,
the harvest -- exists to keep that answer correct without a human re-deriving
it. Launchers are clients of the HTTP API; the reference deployment serves
the Nexira launcher, and the SC-archive importer exists because that is where
the first packs came from, but no component assumes either.

## Components

One repository, two binaries, one SPA:

- **`smrt`** (the service) -- axum HTTP server. Serves the public read API
  (catalog, manifests, mod pages, cached jars), the member/admin authoring
  API, the OpenAPI reference at `/docs`, and the control panel itself. Owns
  the background jobs (builds, harvest scheduling). Deployed as a single
  static binary behind nginx; systemd unit in `deploy/`.
- **`smrt-pack`** (the CLI) -- the same authoring code paths, callable from a
  shell on the box or a workstation: `bootstrap` (seed a config from an SC
  archive), `build`, `depfill`, enrichment passes, `registry` maintenance
  (harvest, stats, classify, conflicts, orphans). Ships to the VPS alongside
  the service so it can never drift behind the registry schema.
- **Panel** (`web/`) -- Svelte 5 SPA served by the service. Curator UI over
  the same HTTP API: pack editor, resolve report, build console, registry
  browser (mods -> releases -> files), moderation and audit views. Talks to
  the API with generated TypeScript bindings, so wire drift is a compile
  error.

Shared by all three:

- **Storage** (`src/storage.rs`) -- the on-disk layout under one root. Plain
  files, atomic writes, no daemon-private state: everything the mirror serves
  is inspectable with `ls` and `cat`.
- **Registry** (`src/registry/`) -- SQLite (WAL) database of mod identity:
  mods, releases, files, aliases, relations, per-jar classification. The
  *decision layer* for side/required derivation reads from here.
- **Authoring** (`src/authoring/`) -- the pipeline: config -> enrichment ->
  classification -> source resolution -> manifest. Plus the harvest (jar
  scanning + Modrinth reconciliation) and depfill (dependency auto-pull).

## Storage tree

```
/var/lib/smrt/
  registry.db                    # the mod-identity registry (SQLite, WAL)
  removed.txt                    # takedown list: sha1s that must never serve again
  featured.json                  # editorial: featured packs/servers
  servers/<id>.json              # curated server metadata
  cache/<xx>/<sha1>.jar          # content-addressed jar cache (xx = first two hex)
  packs/<PackId>/
    summary.json                 # the catalog card (built)
    authoring/config.json        # the curator's declaration (source of truth)
    manifests/<version>.json     # frozen builds
    manifests/latest             # symlink -> the current build
    static/...                   # mirror-hosted pack files (configs, resource packs)
  packs/u/<uid>/<PackId>/...     # community packs, same shape, namespaced by owner
```

Two files matter more than the rest: `authoring/config.json` is what a human
edits (directly or through the panel), and `manifests/<version>.json` is what
a launcher consumes. Everything between them is derived and rebuildable.

## Data flows

### Authoring -> publish

```
config.json --(save/PUT)--> depfill (pull missing hard deps from Modrinth/cache)
config.json --(build)-----> enrichment (mcmod display, inferred requires)
                            classification (registry decision layer: side/policy)
                            source resolution (Modrinth lookups, cache reads)
                            derive_required (seeds + hard-dep walk + invariants)
                            manifest <version>.json + summary.json + latest
```

A build is a *pure function of the config and the registry* plus network
lookups; it writes nothing until the manifest is complete. Real builds of the
same pack are serialized; dry runs (`?dry_run=true`) resolve everything and
publish nothing.

### Harvest cycle

After every real build or cache upload the harvest scheduler is poked (it can
also be forced via `POST /v1/registry/harvest`). A harvest run:

1. reads every cached jar in one pass each (metadata files, bytecode graph,
   icons are extracted on demand elsewhere);
2. reconciles with Modrinth: sha1 -> version lookups, project env flags,
   declared dependencies, identity folds (slug == modid), one-time modid
   learning for re-uploads;
3. rewrites the derived registry layers (packages, inferred + modrinth
   relations, jar classifications) in one transaction. Authored/curator rows
   are precious and never clobbered.

The cycle build -> harvest -> build converges: a build downloads new jars into
the cache, the harvest learns what they are, the next build classifies them.

### Launcher update flow

Covered normatively in [api.md](api.md); in one line: catalog -> pack summary
(`latest_pack_version` / `latest_channel` / `latest_built_at`) -> versions
listing -> manifest -> per-entry download by source -> sha1 verify.

## Trust and roles

Reads are anonymous. Writes are tiered: **Member** (GitHub OAuth; owns
community packs), **Admin** (operator allowlist by GitHub uid, or the
`SMRT_ADMIN_TOKEN` bearer for headless use), **Debug** (a separate token/uid
rung above admin gating compat-affecting registry writes, e.g. authored jar
classification). Frontend role checks are rank-aware; the backend enforces
regardless.

## Design invariants

- **One home per concern.** Instance content is declared in the config,
  identity lives in the registry, presentation hints ride the manifest's
  `display` block. No fact is stored twice on purpose; read-time derivation
  is preferred over duplicated persisted state (e.g. a summary's
  `latest_built_at` is read from the latest manifest, never written).
- **Derived layers are rebuildable.** Everything the harvest writes can be
  wiped and re-derived from jars + Modrinth. Authored rows are the exception
  and are defended in SQL (`WHERE source NOT IN ('curator','authored')`).
- **The client invariant.** A client-side mod is never `required` in a built
  manifest. The build refuses to produce one (see
  [concepts.md](concepts.md)).
- **Self-contained serving.** The panel, the docs page, icons, avatars --
  everything a browser loads comes from the mirror's own origin. Third-party
  fetches happen server-side where they are cacheable and attributable.
- **Takedown-safe.** `removed.txt` blocks a sha1 from serving and from
  re-ingestion, even if the bytes are still on disk.
