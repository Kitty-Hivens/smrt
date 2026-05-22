# Industrial pack ingest scripts

End-to-end tooling that took SmartyCraft's Industrial 1.12.2 pack and shipped it through the smrt mirror as the first real curated pack. Kept here as a worked example of the full pipeline; future packs reuse the same shapes with their own paths and curation tables.

Pipeline overview, in execution order:

1. **`extract-mod-annotations.py`** -- parses every jar's `@Mod` annotation via raw `.class` bytecode (no `javap` dependency). Surfaces `acceptableRemoteVersions` so the curator can tell which mods strict-pin their version on the SC handshake side and which are permissive. The handshake-side reality is what determines whether a smrt_cache substitute can ship a different version than SC's own bytes.

2. **`enrich-mods.py`** -- per-jar `mcmod.info` extraction (description, author, project URL) plus Modrinth sha1 batch lookup and slug-by-modid fallback. Writes `/tmp/industrial-mods-enriched.json` consumed by the pack-config generator.

3. **`upload-mods.sh`** -- bash bulk-uploader that PUTs every jar in `mods/` and `mods/1.12.2/` into the mirror's smrt_cache via the admin API. Reads the admin token from `/tmp/smrt-token`. Substitutes SC's proprietary Smarty mod with the open-smrt-network jar at upload time (drop-in handshake replacement).

4. **`upload-static.py`** -- Python bulk-uploader for the per-pack static assets (configs, resourcepacks, shaderpacks, root client-settings files). Hits `/v1/admin/packs/{id}/static/{rel_path}`; URL-encodes path segments so shaderpack filenames with spaces ("Chocapic13 V7.1 High.zip") work.

5. **`build-pack-config.py`** -- the actual curation step. Reads the SC manifest cache (`~/.local/share/nexira/manifest-cache/Industrial.json`) as the source of truth for which mods are server-required (top-level `mods/`) vs optional pool (`mods/1.12.2/`), merges in the enriched metadata, applies the curation table (drop list, optional overrides like OptiFine -> toggleable, Modrinth-direct cozy additions like AppleSkin and Mizuno's), emits the wire `PackConfig` JSON for `smrt-pack build`.

After the five steps run, `smrt-pack build` on the VPS reads the emitted `PackConfig`, resolves every source (smrt_cache verifies the file is on disk, modrinth fetches version metadata, smrt_static reads the local asset), writes the wire manifest under `/var/lib/smrt/packs/Industrial/manifests/`, atomically swaps the `latest` symlink, and writes the summary that powers `/v1/packs`.

## Paths assumed

- SC pack files on disk under `~/.local/share/nexira/clients/Industrial/`
- SC manifest cache at `~/.local/share/nexira/manifest-cache/Industrial.json`
- Admin token at `/tmp/smrt-token` (chmod 600; never commit)
- SSH access to the live mirror as `root@hivens.dev` with `~/.ssh/vps_hivens`

Most paths are hard-coded; the scripts are example-scoped, not a generic CLI. For another pack, copy the directory and edit the constants at the top of each file.

## License notes that turned up while curating

A scan of the optional pool surfaced a mix of licenses on the assets we host -- worth recording because it shapes which entries the launcher should mark as redistributable in the future Display block:

- `MIT` / `LGPL` / `Apache` / `MPL` -- fully redistributable, no concern
- `CC-BY-NC-SA` -- non-commercial, fine for our non-paid mirror
- `CC-BY-NC-ND` -- no derivatives, hosting verbatim is fine, repackaging is not
- `LicenseRef-All-Rights-Reserved` -- formally no redistribution permission; civil-DMCA risk on hosting. SC has hosted these for years; we inherit the same posture
- Proprietary (OptiFine, Sildurs Vibrant Shaders) -- not on Modrinth, hosted via smrt_cache when SC includes them
