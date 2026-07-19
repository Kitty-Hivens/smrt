# Operations

Running the mirror and curating its content.

## Environment

| Variable | Default | Purpose |
|---|---|---|
| `SMRT_BIND_ADDR` | `127.0.0.1:9000` | TCP bind address (nginx terminates TLS in front). |
| `SMRT_STORAGE_DIR` | `/var/lib/smrt` | Storage root (see the tree in [architecture.md](architecture.md)). |
| `SMRT_MIRROR_BASE` | `http://127.0.0.1:9000` | Public base URL baked into manifest source URLs. Set to your real origin on any public deployment. |
| `SMRT_ADMIN_TOKEN` | none | Bearer for headless admin calls; admin routes refuse without a valid identity. |
| `SMRT_OPERATOR_UID` | `0` | GitHub uid that owns operator-authored packs (and backfills ownership on packs predating the field). |
| `SMRT_GITHUB_CLIENT_ID` / `SMRT_GITHUB_CLIENT_SECRET` | none | GitHub OAuth app for panel sign-in. Absent = OAuth login disabled. |
| `SMRT_ADMIN_GITHUB_UIDS` | empty | Comma-separated GitHub uids granted Admin on sign-in. |
| `SMRT_DEBUG_TOKEN` / `SMRT_DEBUG_GITHUB_UIDS` | none | The Debug rung above Admin: gates compat-affecting registry writes (authored classification, forced overrides). Leave unset in production unless needed. |
| `SMRT_COOKIE_SECURE` | `true` | Set `false` only for plain-HTTP local dev. |
| `RUST_LOG` | `smrt=info` | tracing filter. |

Production config lives in `/etc/smrt/env` (systemd `EnvironmentFile`).

## Deploy

Push to `main` deploys: CI runs the gates, builds both binaries, ships them
via SSH (`smrt` restarts the service; `smrt-pack` is replaced in place -- it
opens the same registry and must never drift behind the schema), and probes
`/v1/health` until it answers. `deploy/` holds the systemd unit, nginx conf,
and the emergency local-deploy script for when Actions is down.

Two operational consequences of a deploy:

- **jobs are in-memory** -- a restart kills running builds and 404s their job
  ids. Do not start long server-side jobs while a deploy is in flight.
- **migrations run at service start** (`registry_meta.schema_version`
  gates them). A failed migration keeps the old schema and refuses further
  steps; fix forward.

Back up `registry.db` before risky curation:

```
smrt-pack registry backup --storage /var/lib/smrt --out registry-$(date +%F).db
```

## The authoring workflow

Day to day, everything happens in the panel; the CLI mirrors it for scripting.

1. **Declare** mods in the pack editor (Modrinth picker, mirror cache picker,
   or GitHub-release ingest which lands the jar in the cache). Set
   `default_enabled` per mod; set the pack's `version` base (`MAJOR.MINOR`).
2. **Save** -- the server runs depfill: missing hard dependencies are pulled
   in (Modrinth first, mirror cache second), the resolved `requires` graph is
   recorded. Depfill failure (an upstream outage) is non-fatal: the config
   saves as-is and the resolve report shows what is missing.
3. **Review the resolve report**: unresolved sources, missing dependencies,
   loader mismatches, side advisories (server-side mods, coremods,
   side disagreements between Modrinth and bytecode), curator suggestions
   from `Recommends` edges.
4. **Build** -- pick a channel (default beta; release is deliberate),
   optionally pin an explicit version. The build classifies, derives
   required-ness, resolves every source, and publishes manifest + summary +
   latest pointer. Dry-run first when in doubt: same resolution, nothing
   published.
5. **Converge** freshly added Modrinth mods: the first build downloads their
   jars; the auto-harvest after it learns identities/env flags; the next
   save + build classifies them fully. Newly pulled dependencies of new mods
   may need one more save -> build cycle.

CLI equivalents: `smrt-pack bootstrap | validate | depfill | build
--channel ... | enrich-mcmod | infer-requires | apply-role-table |
upload-static | reconstruct-config`.

## Harvest

Runs after every real build and cache upload (poked), or on demand:
`POST /v1/registry/harvest` (admin; returns `{running, last_report}`),
`GET /v1/registry/harvest/status`, or `smrt-pack registry harvest`. A run
re-reads every cached jar, reconciles Modrinth identity (sha1 lookups, env
flags incl. a backfill for aliases that predate the env columns, declared
deps, slug==modid folds, one-time modid learning for re-uploads), and
rewrites the derived registry layers. Idempotent; authored rows survive.

Modrinth outages degrade, not break: each metadata call has a hard deadline,
the filtered version listing falls back to unfiltered, 429s are absorbed
once, and failed enrichment legs log a warning and skip. Re-run the harvest
after the outage; everything self-heals.

## Registry curation

The panel's Mods section is the curation surface:

- **Identity** -- assign an unidentified cached jar to a mod (new or
  existing) with version/channel/loaders/MC (`authored` source, precious).
- **Classification** -- the Debug-gated escape hatch for jars whose
  side/policy the automatics get wrong: panel, `PUT
  /v1/registry/files/{sha1}/class`, or `smrt-pack registry classify --sha1
  ... --side ... --policy ...`. Refused for Modrinth-identified mods (their
  env flags win) and for the inconsistent client+must_match pair.
- **Relations** -- authored dependency edges override derived ones
  (e.g. downgrading a false inferred hard edge to optional).
- **Repack provenance** -- a file whose sha1 Modrinth confirms shows
  `Modrinth`; a self-hosted sibling under the same mod shows `repack?` with a
  by-request class-level diff. Nothing is auto-merged or hidden.
- **Takedown** -- removes the jar from serving and records the sha1 in
  `removed.txt`, which also blocks re-ingestion. Manifests referencing it
  must be rebuilt; the moderation policy is to self-host only genuine
  archival jars in the first place.

## Failure modes worth knowing

- **Modrinth partial outages** are the common weather: some endpoints answer,
  others hang or 500. Depfill may pull nothing (config stays as-is), env
  backfills lag, icons vanish from the panel until the CDN returns. All of it
  converges on the next healthy harvest + build.
- **A build during a harvest** classifies against whatever the registry held
  when it started; if env flags landed mid-run, rebuild once the harvest is
  done.
- **Restart mid-build**: the job dies with the process (in-memory); the pack
  lock dies with it too. Re-trigger the build; manifests are only written
  complete.
