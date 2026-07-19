# Public API guide

What a client author needs beyond the endpoint reference. The exact paths,
parameters and response schemas live at `/docs` (Scalar over
`/openapi.json`) on any running mirror; this page carries the semantics that
a schema cannot: flows, ordering rules, compatibility promises.

The reference deployment lives at `https://smrt.hivens.dev`; substitute your
own origin throughout. All public reads are anonymous.

## Schema versioning and forward compatibility

Wire objects carry `schema_version` (currently **2**). Clients must reject a
manifest whose major schema they do not know, and must **ignore unknown
fields** everywhere: the mirror adds optional fields without bumping the
version. Only a wire-incompatible change bumps it.

Practical consequences:

- decode tolerantly (unknown fields, unknown `source` variants, unknown
  `display.presence` values -> no badge, not an error);
- never rely on field order or on absent-vs-null distinctions beyond what the
  schema states;
- treat additive fields as optional forever (a manifest built before a field
  landed simply lacks it).

## The launcher update flow

```
GET /v1/packs                              # catalog: official published packs
GET /v1/packs/{id}                         # one summary (also: latest_* fields)
GET /v1/packs/{id}/manifest/versions       # every retained build, newest first
GET /v1/packs/{id}/manifest                # the latest build
GET /v1/packs/{id}/manifest/{version}      # a specific build
```

1. **Catalog**: `latest_pack_version` on each summary is the current pointer.
   `latest_built_at` (RFC 3339) and `latest_channel` are derived by the mirror
   from the latest manifest at read time -- render "updated X ago" and the
   channel badge from these; absence means the pack has no readable build.
2. **Did anything change?** Compare `latest_pack_version` by plain string
   equality against the installed version. For content-level change detection
   use the manifest `fingerprint`: identical fingerprint = identical
   instance, whatever the labels say.
3. **Version picker**: the versions listing's `builds[]` is newest-first by
   `date_published` and follows the Modrinth version-object naming
   (`version_number`, `version_type`, `date_published`, plus `fingerprint`,
   `mods_count`, `assets_count`). `latest` names the build the latest pointer
   serves. Filter by `version_type` to hide prereleases.
4. **Ordering**, when a client must sort labels itself: numeric tuple
   comparison within a version base (`0.4.10` > `0.4.2`; lexicographic sort
   is wrong); across bases or historical labels, order by `date_published`.

## Downloading an instance

For each `mods[]` / `assets[]` entry, dispatch on `source.type`:

- **`modrinth`**: resolve the actual file via Modrinth
  (`/v2/project/{project_id}/version/{version_id}`), pick the file with
  `primary: true` (fall back to `files[0]` only when nothing is marked
  primary -- Modrinth versions often ship sources/deobf jars alongside the
  installable one). Verify the downloaded bytes against the manifest's
  `sha1`; the manifest, not Modrinth, is the contract.
- **`smrt_cache`**: `source.url` points at
  `/v1/cache/{xx}/{sha1}.jar` on the mirror. Content-addressed and immutable;
  cache aggressively, dedup across packs by sha1.
- **`smrt_static`**: `source.url` points under
  `/v1/packs/{id}/static/...`. Not content-addressed; re-fetch per version
  and verify the manifest's `sha1`.

Install flags: `required` is enforcing (never offer a toggle); for optional
entries `default_enabled` (absent = true) is the install-time default. The
`display` block is advisory UX metadata -- names, descriptions, icons, the
`requires` tree for co-toggling, `presence` for the side badge.

**Toggle identity** across version bumps: key an optional mod's on/off state
by its Modrinth `project_id` when the source is Modrinth, else by the entry's
`slug` field when present, else by `filename`.

## Mods, files, hashes

```
GET /v1/mods/{key}      # key = numeric id | sha1:<hash> | slug
GET /v1/files/{sha1}    # hash-first: file + its release + owning mod
```

The mod page model carries identity (name, slug, modid, Modrinth project id),
the project environment flags (`client_side`/`server_side`, Modrinth
vocabulary, absent for mods without a Modrinth identity), releases with files
(loaders, MC versions, `cached` = the mirror holds the bytes), dependency
edges, and which public packs ship it. `/v1/files/{sha1}` is the Modrinth
`version_file/{hash}` analog: identify an arbitrary jar in one call.

## Icons and images

- `/v1/cache/icon/{sha1}` -- the icon embedded in a cached jar (mcmod.info
  logoFile, mods.toml logoFile, fabric icon, or a conventional root png).
  Immutable-cacheable; 404 when the jar carries none.
- Modrinth-sourced entries may carry `display.icon_url`; when absent, clients
  can resolve the project icon themselves -- and should fall back to the
  jar-embedded icon by sha1 when Modrinth is unreachable (the mirror caches
  the jars either way).
- `/v1/users/{uid}/avatar` -- GitHub avatars proxied through the mirror, so a
  page never hands viewer IPs to a third party.

## Servers, featured, community

`/v1/servers` and `/v1/featured` are curated editorial surfaces for the
launcher's home screen. `/v1/community` lists published community packs (with
an owner byline) -- browseable, but never part of the official catalog at
`/v1/packs`.

## Authenticated surfaces

Not needed by a launcher, listed for completeness: GitHub OAuth session
(`/v1/auth/github/login` -> callback -> cookie; `/v1/me`), member endpoints
(`/v1/me/...`: own packs, uploads, forks), admin authoring
(`/v1/authoring/...`, `/v1/registry/...`; bearer `SMRT_ADMIN_TOKEN` or an
allowlisted OAuth session), and a debug rung above admin for compat-affecting
registry writes. Job endpoints (`/v1/jobs/{id}`, `/v1/jobs/{id}/events` --
SSE) track builds; finished jobs keep answering the status endpoint from
persisted snapshots across restarts (a job running at a restart reads failed,
with an explicit interrupted line), while the live SSE tail is
memory-only.
